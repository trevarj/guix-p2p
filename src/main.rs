mod behaviour;
mod channel;
mod config;
mod daemon;
mod dht;
mod http_client;
mod identity;
mod narinfo;
mod swarm;

use anyhow::Context;
use channel::{SwarmCommand, SwarmNotification};
use clap::{Parser, Subcommand};
use libp2p::{SwarmBuilder, quic, request_response};

#[derive(Parser)]
#[command(name = "guix-p2p-substitute", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,

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

#[derive(Subcommand)]
enum Command {
    /// Run in query mode (driven by guix-daemon --query)
    Query,
    /// Run in substitute mode (driven by guix-daemon --substitute)
    Substitute,
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
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<SwarmCommand>();
    let (notify_tx, notify_rx) = tokio::sync::mpsc::unbounded_channel::<SwarmNotification>();
    let (query_tx, query_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let cache_for_swarm = provider_cache.clone();
    let notify_for_kad = notify_tx.clone();
    let cmd_rx_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(cmd_rx);

    tokio::spawn(async move {
        run_swarm_task(swarm, cache_for_swarm, notify_for_kad, cmd_rx_stream, query_rx).await;
    });

    tracing::info!("Swarm task spawned, entering daemon event loop");

    match cli.command {
        Command::Query => {
            daemon::run_query_mode(
                &provider_cache,
                &query_tx,
                &cmd_tx,
                notify_rx,
                &narinfo_cache,
                &config,
            )
            .await?
        },
        Command::Substitute => {
            daemon::run_substitute_mode(
                &provider_cache,
                &cmd_tx,
                notify_rx,
                &narinfo_cache,
                &config,
                &query_tx,
            )
            .await?
        },
    }

    Ok(())
}

async fn run_swarm_task(
    mut swarm: libp2p::Swarm<behaviour::GuixP2PBehaviour>,
    cache: dht::ProviderCache,
    notify_tx: tokio::sync::mpsc::UnboundedSender<SwarmNotification>,
    mut cmd_rx: tokio_stream::wrappers::UnboundedReceiverStream<SwarmCommand>,
    mut query_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
) {
    use futures::StreamExt;
    use libp2p::swarm::SwarmEvent;

    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(behaviour::GuixP2PEvent::Kad(ref e)) => {
                        dht::handle_kad_event(&cache, &notify_tx, e);
                    },
                    SwarmEvent::Behaviour(behaviour::GuixP2PEvent::BlockExchange(e)) => {
                        handle_block_exchange(&notify_tx, e);
                    },
                    other => handle_swarm_event(other),
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

fn handle_swarm_command(swarm: &mut libp2p::Swarm<behaviour::GuixP2PBehaviour>, cmd: SwarmCommand) {
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
    event: request_response::Event<
        crate::swarm::codec::BlockRequest,
        crate::swarm::codec::BlockResponse,
    >,
) {
    match event {
        request_response::Event::Message { peer, message, .. } => match message {
            request_response::Message::Request { .. } => {
                tracing::trace!("Incoming block request from {}", peer);
            },
            request_response::Message::Response { response, .. } => {
                let _ = notify_tx.send(SwarmNotification::BlockResponse { peer, response });
            },
        },
        other => {
            tracing::trace!("BlockExchange event: {:?}", other);
        },
    }
}

fn build_swarm(
    keypair: &libp2p::identity::Keypair,
) -> anyhow::Result<libp2p::Swarm<behaviour::GuixP2PBehaviour>> {
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

fn handle_swarm_event(event: libp2p::swarm::SwarmEvent<behaviour::GuixP2PEvent>) {
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
        SwarmEvent::Behaviour(behaviour::GuixP2PEvent::Mdns(e)) => match e {
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
        SwarmEvent::Behaviour(behaviour::GuixP2PEvent::Identify(e)) => {
            tracing::debug!("Identify event: {:?}", e);
        },
        SwarmEvent::Behaviour(behaviour::GuixP2PEvent::Kad(_e)) => {
            tracing::trace!("Kademlia event");
        },
        SwarmEvent::Behaviour(behaviour::GuixP2PEvent::BlockExchange(_e)) => {
            tracing::trace!("Block exchange event");
        },
        other => {
            tracing::trace!("Swarm event: {:?}", other);
        },
    }
}
