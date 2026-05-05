use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use guix_p2p_substitute::{
    behaviour::{self, GuixP2PBehaviour, GuixP2PEvent},
    channel::{SwarmCommand, SwarmNotification},
    config,
    connection::{ConnectionConfig, ConnectionManager},
    daemon, dht, identity, narinfo,
    reputation::ReputationTracker,
    swarm::codec::{BlockRequest, BlockResponse},
};
use libp2p::{SwarmBuilder, quic, request_response};

#[derive(Parser)]
#[command(name = "guix-p2p-substitute", version)]
struct Cli {
    /// Run in query mode (driven by guix-daemon --query)
    #[arg(long, conflicts_with_all = ["substitute", "daemon"])]
    query: bool,

    /// Run in substitute mode (driven by guix-daemon --substitute)
    #[arg(long, conflicts_with_all = ["query", "daemon"])]
    substitute: bool,

    /// Run as a persistent background daemon
    #[arg(long, conflicts_with_all = ["query", "substitute"])]
    daemon: bool,

    /// Comma-separated list of bootstrap peer multiaddrs
    #[arg(long, global = true)]
    bootstrap_peers: Option<String>,

    /// Address to listen on (multiaddr format)
    #[arg(long, global = true, default_value = "/ip4/0.0.0.0/udp/6881/quic-v1")]
    listen_addr: Option<String>,

    /// Directory for cache and identity storage
    #[arg(long, global = true)]
    cache_dir: Option<String>,

    /// Comma-separated substitute URLs for HTTP fallback
    #[arg(long, global = true)]
    substitute_urls: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    let config = config::Config::load(
        cli.bootstrap_peers,
        cli.listen_addr,
        cli.cache_dir,
        cli.substitute_urls,
    );

    tracing::info!("Starting guix-p2p-substitute");
    tracing::info!("Cache directory: {}", config.cache_dir.display());

    let keypair = identity::load_or_generate_keypair(&config.cache_dir)
        .context("failed to load or generate identity")?;

    let peer_id = identity::peer_id_from_keypair(&keypair);
    tracing::info!("Peer ID: {}", peer_id);

    let mut swarm = build_swarm(&keypair)?;

    let listen_addr: libp2p::Multiaddr =
        config.listen_addr.parse().context("failed to parse listen address")?;
    swarm.listen_on(listen_addr).context("failed to listen")?;

    dht::bootstrap(&mut swarm, &config.bootstrap_peers)?;

    let provider_cache = dht::create_provider_cache();
    let narinfo_cache = std::sync::Mutex::new(narinfo::NarinfoCache::new(60));

    let conn_config = ConnectionConfig {
        connect_timeout: std::time::Duration::from_secs(config.request_timeout_secs),
        max_retries: config.connection_retries,
        backoff_base: std::time::Duration::from_secs(1),
        health_check_interval: std::time::Duration::from_secs(config.health_check_interval_secs),
        max_peers: config.max_total_peers,
    };
    let rep_path = config.cache_dir.join("reputation.json");
    let reputation = Arc::new(std::sync::Mutex::new(
        ReputationTracker::load(&rep_path, config.reputation_ban_threshold)
            .unwrap_or_else(|_| ReputationTracker::new(config.reputation_ban_threshold)),
    ));
    let conn_mgr = Arc::new(std::sync::Mutex::new(ConnectionManager::new(conn_config)));

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<SwarmCommand>();
    let (notify_tx, notify_rx) = tokio::sync::mpsc::unbounded_channel::<SwarmNotification>();
    let (query_tx, query_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let cache_for_swarm = provider_cache.clone();
    let notify_for_kad = notify_tx.clone();
    let cmd_rx_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(cmd_rx);
    let rep_for_swarm = reputation.clone();
    let conn_for_swarm = conn_mgr.clone();

    tokio::spawn(async move {
        run_swarm_task(
            swarm,
            cache_for_swarm,
            notify_for_kad,
            cmd_rx_stream,
            query_rx,
            rep_for_swarm,
            conn_for_swarm,
        )
        .await;
    });

    tracing::info!("Swarm task spawned, entering daemon event loop");

    if cli.query {
        daemon::run_query_mode(
            &provider_cache,
            &query_tx,
            &cmd_tx,
            notify_rx,
            &narinfo_cache,
            &config,
        )
        .await?
    } else if cli.substitute {
        daemon::run_substitute_mode(
            &provider_cache,
            &cmd_tx,
            notify_rx,
            &narinfo_cache,
            &config,
            &query_tx,
            &reputation,
        )
        .await?
    } else if cli.daemon {
        daemon::run_daemon_mode(
            &provider_cache,
            &cmd_tx,
            notify_rx,
            &narinfo_cache,
            &config,
            &reputation,
            &conn_mgr,
        )
        .await?
    } else {
        tracing::error!("No mode specified. Use --query, --substitute, or --daemon.");
        std::process::exit(1);
    }

    let _ = reputation.lock().unwrap().save(&rep_path);

    Ok(())
}

async fn run_swarm_task(
    mut swarm: libp2p::Swarm<GuixP2PBehaviour>,
    cache: dht::ProviderCache,
    notify_tx: tokio::sync::mpsc::UnboundedSender<SwarmNotification>,
    mut cmd_rx: tokio_stream::wrappers::UnboundedReceiverStream<SwarmCommand>,
    mut query_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    reputation: Arc<std::sync::Mutex<ReputationTracker>>,
    conn_mgr: Arc<std::sync::Mutex<ConnectionManager>>,
) {
    use std::time::Duration;

    use futures::StreamExt;
    use libp2p::swarm::SwarmEvent;

    let mut prune_tick = tokio::time::interval(Duration::from_secs(300));

    loop {
        tokio::select! {
            _ = prune_tick.tick() => {
                let dead = conn_mgr.lock().unwrap().prune_dead();
                if !dead.is_empty() {
                    tracing::debug!("Pruned {} dead connections", dead.len());
                }
                reputation.lock().unwrap().prune_stale(Duration::from_secs(30 * 24 * 3600));
            }
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(GuixP2PEvent::Kad(ref e)) => {
                        dht::handle_kad_event(&cache, &notify_tx, e);
                    },
                    SwarmEvent::Behaviour(GuixP2PEvent::BlockExchange(e)) => {
                        handle_block_exchange(&notify_tx, e, &reputation, &conn_mgr, &mut swarm);
                    },
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        conn_mgr.lock().unwrap().on_connected(peer_id);
                        tracing::debug!("Connection established with {}", peer_id);
                    },
                    SwarmEvent::ConnectionClosed { peer_id, .. } => {
                        conn_mgr.lock().unwrap().on_disconnected(peer_id);
                        tracing::debug!("Connection closed with {}", peer_id);
                    },
                    other => handle_swarm_event(other, &conn_mgr),
                }
            }
            Some(cmd) = cmd_rx.next() => {
                handle_swarm_command(&mut swarm, cmd);
            }
            Some(hash) = query_rx.recv() => {
                let key = libp2p::kad::RecordKey::new(&dht::extract_hash_bytes(&hash));
                swarm.behaviour_mut().kad.get_providers(key);
                tracing::debug!("DHT get_providers for hash={}", hash);
            }
        }
    }
}

fn handle_swarm_command(swarm: &mut libp2p::Swarm<GuixP2PBehaviour>, cmd: SwarmCommand) {
    match cmd {
        SwarmCommand::GetProviders { hash } => {
            let key_bytes = dht::extract_hash_bytes(&hash);
            let key = libp2p::kad::RecordKey::new(&key_bytes);
            swarm.behaviour_mut().kad.get_providers(key);
            tracing::debug!("DHT get_providers for hash={}", hash);
        },
        SwarmCommand::SendBlockRequest { peer, request } => {
            let req_id = swarm.behaviour_mut().block_exchange.send_request(&peer, request);
            tracing::debug!("Sent block request {:?} to peer={}", req_id, peer);
        },
    }
}

fn handle_block_exchange(
    notify_tx: &tokio::sync::mpsc::UnboundedSender<SwarmNotification>,
    event: request_response::Event<BlockRequest, BlockResponse>,
    reputation: &Arc<std::sync::Mutex<ReputationTracker>>,
    conn_mgr: &Arc<std::sync::Mutex<ConnectionManager>>,
    swarm: &mut libp2p::Swarm<GuixP2PBehaviour>,
) {
    match event {
        request_response::Event::Message { peer, message, .. } => match message {
            request_response::Message::Request { request_id, request, channel, .. } => {
                tracing::trace!("Incoming block request from {}", peer);
                if let Some(resp) = serve_block_request(&request) {
                    let _ = swarm.behaviour_mut().block_exchange.send_response(channel, resp);
                    tracing::trace!("Served block request to {}", peer);
                } else {
                    tracing::trace!("Could not serve block request (request_id={})", request_id);
                }
            },
            request_response::Message::Response { response, .. } => {
                let _ = notify_tx.send(SwarmNotification::BlockResponse { peer, response });
                conn_mgr.lock().unwrap().on_active(peer);
                reputation.lock().unwrap().record_success(peer, 0);
            },
        },
        request_response::Event::OutboundFailure { peer, error, .. } => {
            tracing::warn!("Outbound request failed for {}: {:?}", peer, error);
            reputation.lock().unwrap().record_failure(peer);
        },
        other => {
            tracing::trace!("BlockExchange event: {:?}", other);
        },
    }
}

fn serve_block_request(request: &BlockRequest) -> Option<BlockResponse> {
    match request {
        BlockRequest::Handshake { nar_hash } => {
            let hash_hex = hex::encode(nar_hash);
            tracing::info!("Handshake request for nar_hash={}", hash_hex);

            Some(BlockResponse::HandshakeReply {
                blocks_available: vec![],
                block_count: 0,
                block_size: 262144,
                block_hashes: vec![],
            })
        },
        BlockRequest::GetBlocks { indices: _ } => {
            tracing::trace!("Block request for indices, but local storage not implemented");
            Some(BlockResponse::Error { message: "blocks not available".into() })
        },
    }
}

fn build_swarm(
    keypair: &libp2p::identity::Keypair,
) -> anyhow::Result<libp2p::Swarm<GuixP2PBehaviour>> {
    let mut quic_config = quic::Config::new(keypair);
    quic_config.max_idle_timeout = 30_000;

    let swarm = SwarmBuilder::with_existing_identity(keypair.clone())
        .with_tokio()
        .with_quic_config(|_| quic_config)
        .with_dns()?
        .with_behaviour(|keypair| Ok(behaviour::create_swarm_behaviour(keypair)))?
        .build();

    Ok(swarm)
}

fn handle_swarm_event(
    event: libp2p::swarm::SwarmEvent<GuixP2PEvent>,
    _conn_mgr: &Arc<std::sync::Mutex<ConnectionManager>>,
) {
    use libp2p::swarm::SwarmEvent;
    match event {
        SwarmEvent::NewListenAddr { address, .. } => {
            tracing::info!("Swarm listening on {}", address);
        },
        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            tracing::debug!("Connection established with {}", peer_id);
        },
        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            tracing::debug!("Connection closed with {}", peer_id);
        },
        SwarmEvent::Behaviour(GuixP2PEvent::Mdns(e)) => match e {
            libp2p::mdns::Event::Discovered(list) => {
                for (peer_id, addr) in list {
                    tracing::info!("Discovered LAN peer {} at {}", peer_id, addr);
                }
            },
            libp2p::mdns::Event::Expired(list) => {
                for (peer_id, addr) in list {
                    tracing::debug!("LAN peer expired {} at {}", peer_id, addr);
                }
            },
        },
        SwarmEvent::Behaviour(GuixP2PEvent::Identify(e)) => {
            tracing::debug!("Identify event: {:?}", e);
        },
        SwarmEvent::Behaviour(GuixP2PEvent::Kad(_e)) => {
            tracing::trace!("Kademlia event");
        },
        SwarmEvent::Behaviour(GuixP2PEvent::BlockExchange(_e)) => {
            tracing::trace!("Block exchange event");
        },
        other => {
            tracing::trace!("Swarm event: {:?}", other);
        },
    }
}
