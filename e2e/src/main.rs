use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use clap::Parser;
use futures::StreamExt;
use guix_p2p::{
    behaviour::{GuixP2PBehaviour, GuixP2PEvent, create_swarm_behaviour},
    channel::SwarmCommand,
    connection::{ConnectionConfig, ConnectionManager},
    dashboard::{self, BuildRegistry, DashboardEvent, ObservedBuild},
    dht::{self, ProviderCache},
    nar_store::NarStore,
    reputation::ReputationTracker,
    swarm::{
        block::compute_block_hashes,
        codec::{BlockData, BlockRequest, BlockResponse},
    },
};
use libp2p::{
    Multiaddr, PeerId, SwarmBuilder,
    kad::{self, GetProvidersOk, QueryResult, RecordKey},
    request_response,
    swarm::SwarmEvent,
};
use sha2::Digest;
use tokio::sync::{mpsc::unbounded_channel, oneshot};
use tokio_stream::wrappers::UnboundedReceiverStream;

type BlockMap = Arc<std::sync::Mutex<HashMap<String, Vec<u8>>>>;

// ── CLI ──────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "guix-p2p-e2e", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Launch a multi-node test network with web dashboards
    Run {
        /// Number of seeder nodes (each seeds one synthetic nar)
        #[arg(long, default_value = "2")]
        seeders: usize,
        /// Number of downloader nodes
        #[arg(long, default_value = "1")]
        downloaders: usize,
        /// Starting port for dashboards
        #[arg(long, default_value = "3031")]
        dashboard_port: u16,
        /// Size of synthetic nars in KB
        #[arg(long, default_value = "256")]
        nar_kb: usize,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "guix_p2p_e2e=info,guix_p2p=info,info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.cmd {
        Commands::Run { seeders, downloaders, dashboard_port, nar_kb } => {
            run_network(seeders, downloaders, dashboard_port, nar_kb).await
        },
    }
}

// ── Network orchestrator ──────────────────────────────────────────────

async fn run_network(
    seeder_count: usize,
    downloader_count: usize,
    base_port: u16,
    nar_kb: usize,
) -> anyhow::Result<()> {
    let block_size = 65536;
    let nar_size = nar_kb * 1024;

    let total = seeder_count + downloader_count;
    println!();
    tracing::info!("guix-p2p e2e test network");
    tracing::info!("  seeders: {seeder_count}  downloaders: {downloader_count}  total: {total}");
    tracing::info!("  nar size: {nar_kb} KB  block size: {} KB", block_size / 1024);
    println!();

    // Generate synthetic nars for seeders
    let nars: Vec<(String, Vec<u8>)> = (0..seeder_count)
        .map(|i| {
            let data = make_synthetic_nar(i as u8, nar_size);
            let hash = hex::encode(sha2::Sha256::digest(&data));
            (hash, data)
        })
        .collect();

    // Launch seeder nodes
    let mut seeder_handles = Vec::new();
    let mut seeder_addrs = Vec::new();
    let mut seeder_pids = Vec::new();

    for (i, (hash, data)) in nars.iter().enumerate() {
        let port = base_port + i as u16;
        let (node, addr_rx) = launch_node(
            Some(data.clone()),
            hash.clone(),
            None,
            block_size,
            port,
            format!("seeder-{i}"),
        )
        .await?;

        let addr = addr_rx.await.context("seeder did not report address")?;
        tracing::info!(
            "seeder-{i}  pid={}  addr={}  dash=http://127.0.0.1:{port}",
            node.peer_id,
            addr
        );
        seeder_pids.push(node.peer_id);
        seeder_handles.push(node);
        seeder_addrs.push(addr.clone());
    }

    // Launch downloader nodes
    let mut downloader_handles = Vec::new();
    for i in 0..downloader_count {
        let port = base_port + seeder_count as u16 + i as u16;
        let (node, addr_rx) = launch_node(
            None,
            String::new(),
            Some(seeder_addrs.clone()),
            block_size,
            port,
            format!("dl-{i}"),
        )
        .await?;

        let addr = addr_rx.await.context("downloader did not report address")?;
        tracing::info!(
            "dl-{i}      pid={}  addr={}  dash=http://127.0.0.1:{port}",
            node.peer_id,
            addr
        );
        downloader_handles.push(node);
    }

    // Wait for initial connections
    tracing::info!("waiting for nodes to connect...");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Trigger downloads from downloader nodes
    for (i, dl) in downloader_handles.iter().enumerate() {
        for (j, (hash, _)) in nars.iter().enumerate() {
            let seeder_pid = seeder_pids[j % seeder_pids.len()];
            let block_count = (nar_size).div_ceil(block_size) as u32;

            tracing::info!("dl-{i} -> seeder-{j}: handshake for {}..", &hash[..16]);
            let _ = dl.cmd_tx.send(SwarmCommand::SendBlockRequest {
                peer: seeder_pid,
                request: BlockRequest::Handshake { nar_hash: hex::decode(hash).unwrap() },
            });

            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            tracing::info!("dl-{i} -> seeder-{j}: requesting {block_count} blocks");
            let _ = dl.cmd_tx.send(SwarmCommand::SendBlockRequest {
                peer: seeder_pid,
                request: BlockRequest::GetBlocks { indices: (0u32..block_count).collect() },
            });

            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    println!();
    tracing::info!("network is running. open any dashboard URL above to watch.");
    tracing::info!("press Ctrl-C to stop");
    println!();

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    Ok(())
}

// ── Node launcher ─────────────────────────────────────────────────────

struct NodeHandle {
    peer_id: PeerId,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<SwarmCommand>,
}

async fn launch_node(
    seed_data: Option<Vec<u8>>,
    seed_hash: String,
    bootstrap: Option<Vec<Multiaddr>>,
    block_size: usize,
    dashboard_port: u16,
    label: String,
) -> anyhow::Result<(NodeHandle, oneshot::Receiver<Multiaddr>)> {
    let kp = libp2p::identity::Keypair::generate_ed25519();
    let pid = PeerId::from(kp.public());
    let mut swarm = build_swarm(&kp)?;
    swarm.listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse()?)?;

    // Set up nar store
    let tmp_dir = tempfile::tempdir()?;
    let cache_dir = tmp_dir.path().to_path_buf();
    // Keep the temp dir alive for the duration of the node
    let _tmp_arc: Arc<tempfile::TempDir> = Arc::new(tmp_dir);

    let nar_store = Arc::new(std::sync::Mutex::new(NarStore::new(&cache_dir, block_size)));
    let mut block_map: HashMap<String, Vec<u8>> = HashMap::new();

    // If we have seed data, save it to the nar store and announce in DHT
    if let Some(ref data) = seed_data {
        let mut store = nar_store.lock().unwrap();
        store.save(&seed_hash, data)?;
        block_map.insert(seed_hash.clone(), data.clone());
        drop(store);

        let bytes = hex::decode(&seed_hash).context("invalid seed hash")?;
        swarm.behaviour_mut().kad.start_providing(RecordKey::new(&bytes))?;
    }

    let blocks: BlockMap = Arc::new(std::sync::Mutex::new(block_map));
    let provider_cache = dht::create_provider_cache();
    let rep = Arc::new(std::sync::Mutex::new(ReputationTracker::new(5)));
    let conn = Arc::new(std::sync::Mutex::new(ConnectionManager::new(ConnectionConfig::default())));
    let build_reg: BuildRegistry = Arc::new(std::sync::Mutex::new(HashMap::new()));
    let (event_tx, _) = tokio::sync::broadcast::channel::<DashboardEvent>(256);
    let (cmd_tx, cmd_rx) = unbounded_channel::<SwarmCommand>();
    let (addr_tx, addr_rx) = oneshot::channel::<Multiaddr>();

    // Populate build registry for seed
    if let Some(ref data) = seed_data {
        let mut reg = build_reg.lock().unwrap();
        reg.entry(seed_hash.clone()).or_insert_with(|| ObservedBuild {
            nar_hash: seed_hash.clone(),
            store_path: Some(format!("/gnu/store/{}-synthetic-pkg", &seed_hash[..32])),
            nar_size: Some(data.len() as u64),
            references: vec![],
            deriver: None,
            narinfo_raw: None,
            providers: vec![],
            downloaded_at: None,
            download_size: None,
        });
    }

    // Dashboard HTTP server
    let dash_state = dashboard::DashboardState {
        provider_cache: provider_cache.clone(),
        reputation: rep.clone(),
        conn_mgr: conn.clone(),
        build_registry: build_reg.clone(),
        started: std::time::Instant::now(),
        peer_id: pid.to_string(),
        event_bus: event_tx.clone(),
        nar_store: nar_store.clone(),
    };

    let dash_label = label.clone();
    tokio::spawn(async move {
        dashboard::serve(dash_state, dashboard_port, "127.0.0.1").await;
    });

    // Swarm event loop
    let cmd_stream = UnboundedReceiverStream::new(cmd_rx);
    let c_cache = provider_cache.clone();
    let c_rep = rep.clone();
    let c_conn = conn.clone();
    let c_evt = event_tx.clone();
    let c_blocks = blocks.clone();
    let c_nar = nar_store.clone();
    let c_bootstrap = bootstrap.unwrap_or_default();

    tokio::spawn(async move {
        run_node_loop(
            swarm,
            c_cache,
            c_rep,
            c_conn,
            c_evt,
            c_blocks,
            c_nar,
            addr_tx,
            cmd_stream,
            c_bootstrap,
            block_size,
            &dash_label,
        )
        .await;
    });

    // Keep temp dir alive
    std::mem::forget(_tmp_arc);

    Ok((NodeHandle { peer_id: pid, cmd_tx }, addr_rx))
}

#[allow(clippy::too_many_arguments)]
async fn run_node_loop(
    mut swarm: libp2p::Swarm<GuixP2PBehaviour>,
    cache: ProviderCache,
    rep: Arc<std::sync::Mutex<ReputationTracker>>,
    conn: Arc<std::sync::Mutex<ConnectionManager>>,
    evt: dashboard::EventBus,
    blocks: BlockMap,
    nar_store: Arc<std::sync::Mutex<NarStore>>,
    addr_tx: oneshot::Sender<Multiaddr>,
    mut cmds: UnboundedReceiverStream<SwarmCommand>,
    bootstrap: Vec<Multiaddr>,
    block_size: usize,
    label: &str,
) {
    let mut addr_opt = Some(addr_tx);

    loop {
        tokio::select! {
            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    if let Some(atx) = addr_opt.take() {
                        let full = address.clone().with(
                            libp2p::multiaddr::Protocol::P2p(*swarm.local_peer_id()),
                        );
                        let _ = atx.send(full);
                        for a in &bootstrap {
                            let _ = swarm.dial(a.clone());
                        }
                    }
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    conn.lock().unwrap().on_connected(peer_id);
                    rep.lock().unwrap().record_success(peer_id, 0);
                    let _ = evt.send(DashboardEvent::PeerConnected {
                        peer_id: peer_id.to_string(),
                        addresses: vec![],
                    });
                    if let Err(e) = swarm.behaviour_mut().kad.bootstrap() {
                        tracing::debug!("{}: kad bootstrap: {}", label, e);
                    }
                    tracing::info!("{}: connected to {}", label, peer_id);
                }
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    conn.lock().unwrap().on_disconnected(peer_id);
                    let _ = evt.send(DashboardEvent::PeerDisconnected {
                        peer_id: peer_id.to_string(),
                    });
                }
                SwarmEvent::Behaviour(GuixP2PEvent::Kad(
                    kad::Event::OutboundQueryProgressed {
                        result: QueryResult::GetProviders(Ok(
                            GetProvidersOk::FoundProviders { key, providers, .. },
                        )),
                        ..
                    },
                )) => {
                    let kh = hex::encode(key.as_ref());
                    cache.lock().await.insert(kh.clone(), providers.iter().copied().collect());
                    let short = kh[..16.min(kh.len())].to_string();
                    let _ = evt.send(DashboardEvent::ProvidersFound {
                        nar_hash: kh,
                        provider_count: providers.len(),
                    });
                    tracing::info!("{}: DHT {} provider(s) for {}...", label, providers.len(), short);
                }
                SwarmEvent::Behaviour(GuixP2PEvent::BlockExchange(
                    request_response::Event::Message {
                        peer,
                        message: request_response::Message::Request { request, channel, .. },
                        ..
                    },
                )) => {
                    let nar_hash_event = match &request {
                        BlockRequest::Handshake { nar_hash } => hex::encode(nar_hash),
                        BlockRequest::GetBlocks { .. } => String::new(),
                    };
                    let indices_event = match &request {
                        BlockRequest::GetBlocks { indices } => indices.clone(),
                        BlockRequest::Handshake { .. } => vec![],
                    };
                    let resp = {
                        let store = nar_store.lock().unwrap();
                        if store.has_nar(&nar_hash_event) || matches!(&request, BlockRequest::Handshake { .. }) {
                            store.handle_request(&request)
                        } else {
                            serve_from_blocks(&blocks, &request, block_size)
                        }
                    };
                    if let Some(resp) = resp {
                        let _ = swarm.behaviour_mut().block_exchange.send_response(channel, resp);
                        let _ = evt.send(DashboardEvent::BlockServed {
                            nar_hash: nar_hash_event,
                            peer_id: peer.to_string(),
                            indices: indices_event,
                        });
                    }
                }
                SwarmEvent::Behaviour(GuixP2PEvent::BlockExchange(
                    request_response::Event::Message {
                        peer,
                        message: request_response::Message::Response { response, .. },
                        ..
                    },
                )) => {
                    handle_block_response(&peer, &response, &evt, &nar_store);
                }
                _ => {}
            },
            Some(cmd) = cmds.next() => match cmd {
                SwarmCommand::GetProviders { hash } => {
                    if let Ok(bytes) = hex::decode(hash) {
                        swarm.behaviour_mut().kad.get_providers(RecordKey::new(&bytes));
                    }
                }
                SwarmCommand::SendBlockRequest { peer, request } => {
                    let _ = swarm.behaviour_mut().block_exchange.send_request(&peer, request);
                }
                SwarmCommand::StartProviding { hash } => {
                    if let Ok(bytes) = hex::decode(hash) {
                        let _ = swarm.behaviour_mut().kad.start_providing(RecordKey::new(&bytes));
                    }
                }
            },
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

fn make_synthetic_nar(seed: u8, size: usize) -> Vec<u8> {
    let mut data = vec![seed; size];
    for (i, b) in data.iter_mut().enumerate() {
        *b = seed.wrapping_add((i % 251) as u8);
    }
    data
}

fn build_swarm(kp: &libp2p::identity::Keypair) -> anyhow::Result<libp2p::Swarm<GuixP2PBehaviour>> {
    let mut cfg = libp2p::quic::Config::new(kp);
    cfg.max_idle_timeout = 30_000;
    Ok(SwarmBuilder::with_existing_identity(kp.clone())
        .with_tokio()
        .with_quic_config(|_| cfg)
        .with_dns()?
        .with_behaviour(|kp| Ok(create_swarm_behaviour(kp)))?
        .build())
}

fn serve_from_blocks(
    blocks: &BlockMap,
    req: &BlockRequest,
    block_size: usize,
) -> Option<BlockResponse> {
    match req {
        BlockRequest::Handshake { nar_hash } => {
            let key = hex::encode(nar_hash);
            if let Some(data) = blocks.lock().unwrap().get(&key) {
                let hashes = compute_block_hashes(data, block_size);
                let n = hashes.len() as u32;
                Some(BlockResponse::HandshakeReply {
                    blocks_available: (0..n).collect(),
                    block_count: n,
                    block_size: block_size as u32,
                    block_hashes: hashes.iter().map(|h| h.to_vec()).collect(),
                })
            } else {
                Some(BlockResponse::HandshakeReply {
                    blocks_available: vec![],
                    block_count: 0,
                    block_size: block_size as u32,
                    block_hashes: vec![],
                })
            }
        },
        BlockRequest::GetBlocks { indices } => {
            if let Some(data) = blocks.lock().unwrap().values().next() {
                let blks: Vec<BlockData> = indices
                    .iter()
                    .filter_map(|&i| {
                        let off = i as usize * block_size;
                        (off < data.len()).then(|| {
                            let end = (off + block_size).min(data.len());
                            BlockData { index: i, data: data[off..end].to_vec() }
                        })
                    })
                    .collect();
                Some(BlockResponse::Blocks { data: blks })
            } else {
                Some(BlockResponse::Error { message: "no blocks seeded".into() })
            }
        },
    }
}

fn handle_block_response(
    peer: &PeerId,
    resp: &BlockResponse,
    evt: &dashboard::EventBus,
    nar_store: &Arc<std::sync::Mutex<NarStore>>,
) {
    match resp {
        BlockResponse::HandshakeReply { blocks_available, .. } => {
            tracing::info!(
                "<- handshake reply from {}: {} blocks available",
                peer,
                blocks_available.len(),
            );
        },
        BlockResponse::Blocks { data } => {
            let cnt = data.len();
            let total: usize = data.iter().map(|b| b.data.len()).sum();
            tracing::info!(
                "<- received {cnt} blocks ({:.1} KB) from {}",
                total as f64 / 1024.0,
                peer
            );
            for _bd in data {
                let _ = evt.send(DashboardEvent::BlockReceived {
                    nar_hash: String::new(),
                    peer_id: peer.to_string(),
                    blocks: 1,
                });
            }
        },
        BlockResponse::Error { message } => {
            tracing::warn!("block error from {}: {}", peer, message);
        },
    }
    let _ = nar_store;
}
