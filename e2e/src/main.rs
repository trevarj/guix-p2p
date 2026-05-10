use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::Context;
use clap::{Parser, ValueEnum};
use futures::StreamExt;
use guix_p2p::{
    behaviour::{GuixP2PBehaviour, GuixP2PEvent, create_swarm_behaviour_without_mdns},
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
    noise, request_response,
    swarm::SwarmEvent,
    tcp, yamux,
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
    /// Seed real store paths from /gnu/store/ and serve via P2P
    Seed {
        /// Comma-separated /gnu/store/ paths to seed
        #[arg(long)]
        paths: String,
        /// Dashboard port
        #[arg(long, default_value = "3031")]
        dashboard_port: u16,
        /// Connect to a seeder at this multiaddr (for downloaders)
        #[arg(long)]
        connect: Option<String>,
    },
    /// Run a real Guix p2p-only substitute smoke test
    ContainerSmoke {
        /// Guix package to build through the isolated daemon
        #[arg(long, default_value = "hello")]
        package: String,
        /// Pre-resolved /gnu/store path to seed instead of resolving the package first
        #[arg(long)]
        store_path: Option<String>,
        /// P2P transport for the two local nodes
        #[arg(long, value_enum, default_value_t = HarnessTransport::Tcp)]
        transport: HarnessTransport,
        /// Base directory for generated state and logs
        #[arg(long, default_value = "/tmp/guix-p2p-e2e")]
        base: PathBuf,
        /// Node A libp2p listen port
        #[arg(long, default_value_t = 6881)]
        node_a_port: u16,
        /// Node B libp2p listen port
        #[arg(long, default_value_t = 6882)]
        node_b_port: u16,
        /// Node A dashboard port
        #[arg(long, default_value_t = 3031)]
        node_a_dashboard_port: u16,
        /// Node B dashboard port
        #[arg(long, default_value_t = 3032)]
        node_b_dashboard_port: u16,
        /// Bind address for Node A and Node B dashboards
        #[arg(long, default_value = "127.0.0.1")]
        dashboard_bind: String,
        /// Existing guix-p2p binary to use instead of target/release/guix-p2p
        #[arg(long)]
        guix_p2p_bin: Option<PathBuf>,
        /// Run processes directly because a disposable VM is the isolation boundary
        #[arg(long)]
        vm_direct: bool,
        /// Keep daemons and dashboards running after validation until Ctrl-C
        #[arg(long)]
        hold: bool,
        /// Keep generated state under --base after completion
        #[arg(long)]
        keep_temp: bool,
    },
    /// Run controlled local substitute benchmarks
    Benchmark {
        /// Comma-separated Guix package names
        #[arg(long, value_delimiter = ',', default_value = "hello,git,emacs")]
        packages: Vec<String>,
        /// Benchmark modes
        #[arg(long, value_enum, value_delimiter = ',', default_value = "http,p2p-only,p2p-first")]
        modes: Vec<BenchmarkMode>,
        /// Iterations per package/mode
        #[arg(long, default_value_t = 3)]
        iterations: usize,
        /// P2P transport for P2P benchmark modes
        #[arg(long, value_enum, default_value_t = HarnessTransport::Tcp)]
        transport: HarnessTransport,
        /// Benchmark output and temporary state directory
        #[arg(long, default_value = "target/guix-p2p-bench")]
        base: PathBuf,
        /// Existing guix-p2p binary to use instead of target/release/guix-p2p
        #[arg(long)]
        guix_p2p_bin: Option<PathBuf>,
        /// Keep per-run temporary state directories
        #[arg(long)]
        keep_temp: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum HarnessTransport {
    Tcp,
    Quic,
}

impl HarnessTransport {
    fn listen_addr(self, port: u16) -> String {
        match self {
            HarnessTransport::Tcp => format!("/ip4/127.0.0.1/tcp/{port}"),
            HarnessTransport::Quic => format!("/ip4/127.0.0.1/udp/{port}/quic-v1"),
        }
    }
}

impl std::fmt::Display for HarnessTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessTransport::Tcp => write!(f, "tcp"),
            HarnessTransport::Quic => write!(f, "quic"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum BenchmarkMode {
    Http,
    #[value(name = "p2p-only")]
    P2pOnly,
    #[value(name = "p2p-first")]
    P2pFirst,
}

impl BenchmarkMode {
    fn as_policy(self) -> Option<&'static str> {
        match self {
            BenchmarkMode::Http => None,
            BenchmarkMode::P2pOnly => Some("p2p-only"),
            BenchmarkMode::P2pFirst => Some("p2p-first"),
        }
    }
}

impl std::fmt::Display for BenchmarkMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchmarkMode::Http => write!(f, "http"),
            BenchmarkMode::P2pOnly => write!(f, "p2p-only"),
            BenchmarkMode::P2pFirst => write!(f, "p2p-first"),
        }
    }
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
        Commands::Seed { paths, dashboard_port, connect } => {
            run_seed(&paths, dashboard_port, connect.as_deref()).await
        },
        Commands::ContainerSmoke {
            package,
            store_path,
            transport,
            base,
            node_a_port,
            node_b_port,
            node_a_dashboard_port,
            node_b_dashboard_port,
            dashboard_bind,
            guix_p2p_bin,
            vm_direct,
            hold,
            keep_temp,
        } => {
            run_container_smoke(ContainerSmokeOptions {
                package,
                store_path,
                transport,
                base,
                node_a_port,
                node_b_port,
                node_a_dashboard_port,
                node_b_dashboard_port,
                dashboard_bind,
                guix_p2p_bin,
                vm_direct,
                hold,
                keep_temp,
            })
            .await
        },
        Commands::Benchmark {
            packages,
            modes,
            iterations,
            transport,
            base,
            guix_p2p_bin,
            keep_temp,
        } => {
            run_benchmark(BenchmarkOptions {
                packages,
                modes,
                iterations,
                transport,
                base,
                guix_p2p_bin,
                keep_temp,
            })
            .await
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
            let store_path = synthetic_store_path(hash);
            record_synthetic_download_started(dl, hash, &store_path, nar_size as u64);

            tracing::info!("dl-{i} -> seeder-{j}: handshake for {}..", &hash[..16]);
            let _ = dl.cmd_tx.send(SwarmCommand::SendBlockRequest {
                peer: seeder_pid,
                request: BlockRequest::Handshake { nar_hash: hex::decode(hash).unwrap() },
            });

            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            tracing::info!("dl-{i} -> seeder-{j}: requesting {block_count} blocks");
            let _ = dl.cmd_tx.send(SwarmCommand::SendBlockRequest {
                peer: seeder_pid,
                request: BlockRequest::GetBlocks {
                    nar_hash: hex::decode(hash).unwrap(),
                    indices: (0u32..block_count).collect(),
                },
            });

            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            record_synthetic_download_succeeded(dl, hash, &store_path, nar_size as u64);
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
    build_registry: BuildRegistry,
    event_tx: dashboard::EventBus,
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
    swarm.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?;

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
        catalog: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
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

    Ok((NodeHandle { peer_id: pid, cmd_tx, build_registry: build_reg, event_tx }, addr_rx))
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
                        BlockRequest::GetBlocks { nar_hash, .. } => hex::encode(nar_hash),
                    };
                    let indices_event = match &request {
                        BlockRequest::GetBlocks { indices, .. } => indices.clone(),
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

fn synthetic_store_path(hash: &str) -> String {
    format!("/gnu/store/{}-synthetic-pkg", &hash[..32.min(hash.len())])
}

fn record_synthetic_download_started(
    node: &NodeHandle,
    hash: &str,
    store_path: &str,
    nar_size: u64,
) {
    node.build_registry.lock().unwrap().entry(hash.to_string()).or_insert_with(|| ObservedBuild {
        nar_hash: hash.to_string(),
        store_path: Some(store_path.to_string()),
        nar_size: Some(nar_size),
        references: vec![],
        deriver: None,
        narinfo_raw: None,
        providers: vec![],
        downloaded_at: None,
        download_size: None,
    });

    let _ = node.event_tx.send(DashboardEvent::CatalogEntry {
        hash_part: store_hash_part(store_path).unwrap_or_else(|| hash[..32.min(hash.len())].into()),
        store_path: Some(store_path.to_string()),
        nar_size: Some(nar_size),
        nar_hash: Some(hash.to_string()),
        p2p_available: true,
    });
    let _ = node.event_tx.send(DashboardEvent::DownloadStarted {
        nar_hash: hash.to_string(),
        store_path: store_path.to_string(),
        nar_size,
    });
}

fn record_synthetic_download_succeeded(
    node: &NodeHandle,
    hash: &str,
    store_path: &str,
    nar_size: u64,
) {
    if let Some(build) = node.build_registry.lock().unwrap().get_mut(hash) {
        build.downloaded_at = Some(unix_timestamp_secs());
        build.download_size = Some(nar_size);
    }

    let _ = node.event_tx.send(DashboardEvent::DownloadSucceeded {
        nar_hash: hash.to_string(),
        store_path: store_path.to_string(),
        size: nar_size,
        elapsed_ms: 700,
    });
}

fn build_swarm(kp: &libp2p::identity::Keypair) -> anyhow::Result<libp2p::Swarm<GuixP2PBehaviour>> {
    let mut cfg = libp2p::quic::Config::new(kp);
    cfg.max_idle_timeout = 30_000;
    Ok(SwarmBuilder::with_existing_identity(kp.clone())
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_quic_config(|_| cfg)
        .with_behaviour(|kp| Ok(create_swarm_behaviour_without_mdns(kp)))?
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
        BlockRequest::GetBlocks { nar_hash, indices } => {
            let key = hex::encode(nar_hash);
            if let Some(data) = blocks.lock().unwrap().get(&key) {
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

// ── Seed command: seed real store paths from /gnu/store/ ────────────────

async fn run_seed(
    paths_str: &str,
    dashboard_port: u16,
    connect_addr: Option<&str>,
) -> anyhow::Result<()> {
    let block_size = 262144;
    let paths: Vec<&str> = paths_str.split(',').map(str::trim).collect();

    println!();
    tracing::info!("guix-p2p e2e seed mode");
    tracing::info!("  seeding {} store path(s)", paths.len());

    // Create nar store and seed each path using guix archive --export
    let tmp_dir = tempfile::tempdir()?;
    let cache_dir = tmp_dir.path().to_path_buf();
    let nar_store = Arc::new(std::sync::Mutex::new(NarStore::new(&cache_dir, block_size)));

    let mut block_map: HashMap<String, Vec<u8>> = HashMap::new();
    let mut seeder_hashes: Vec<String> = Vec::new();

    for path in &paths {
        tracing::info!("seeding {}...", path);
        let mut store = nar_store.lock().unwrap();
        match store.seed_store_path(path) {
            Ok(hash) => {
                let info = store.seed_info(&hash).unwrap_or(guix_p2p::nar_store::SeededNarInfo {
                    nar_size: 0,
                    block_count: 0,
                    block_size: block_size as u32,
                });
                tracing::info!(
                    "  hash={}.. size={} blocks={}",
                    &hash[..16],
                    info.nar_size,
                    info.block_count
                );
                // Also populate the simple block_map for direct serving
                let data_path = cache_dir.join("nar").join(format!("{}.nar", hash));
                if let Ok(data) = std::fs::read(&data_path) {
                    block_map.insert(hash.clone(), data);
                }
                seeder_hashes.push(hash);
            },
            Err(e) => {
                tracing::error!("  failed to seed {}: {}", path, e);
            },
        }
    }

    // Keep temp dir alive
    std::mem::forget(tmp_dir);

    // Build the swarm
    let kp = libp2p::identity::Keypair::generate_ed25519();
    let pid = PeerId::from(kp.public());
    let mut swarm = build_swarm(&kp)?;
    swarm.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?;

    // Announce all hashes in DHT
    for hash in &seeder_hashes {
        if let Ok(bytes) = hex::decode(hash) {
            let _ = swarm.behaviour_mut().kad.start_providing(RecordKey::new(&bytes));
        }
    }

    let blocks: BlockMap = Arc::new(std::sync::Mutex::new(block_map));
    let provider_cache = dht::create_provider_cache();
    let rep = Arc::new(std::sync::Mutex::new(ReputationTracker::new(5)));
    let conn = Arc::new(std::sync::Mutex::new(ConnectionManager::new(ConnectionConfig::default())));
    let build_reg: BuildRegistry = Arc::new(std::sync::Mutex::new(HashMap::new()));
    let (event_tx, _) = tokio::sync::broadcast::channel::<DashboardEvent>(256);
    let (_cmd_tx, cmd_rx) = unbounded_channel::<SwarmCommand>();
    let (addr_tx, addr_rx) = oneshot::channel::<Multiaddr>();

    // Populate build registry for seeded items
    for (i, hash) in seeder_hashes.iter().enumerate() {
        let store_path = paths.get(i).unwrap_or(&"").to_string();
        let sz = {
            let s = nar_store.lock().unwrap();
            s.seed_info(hash).map(|i| i.nar_size).unwrap_or(0)
        };
        build_reg.lock().unwrap().entry(hash.clone()).or_insert_with(|| ObservedBuild {
            nar_hash: hash.clone(),
            store_path: Some(store_path),
            nar_size: Some(sz),
            references: vec![],
            deriver: None,
            narinfo_raw: None,
            providers: vec![],
            downloaded_at: None,
            download_size: None,
        });
    }

    // Dashboard
    let dash_state = dashboard::DashboardState {
        provider_cache: provider_cache.clone(),
        reputation: rep.clone(),
        conn_mgr: conn.clone(),
        build_registry: build_reg.clone(),
        started: std::time::Instant::now(),
        peer_id: pid.to_string(),
        event_bus: event_tx.clone(),
        nar_store: nar_store.clone(),
        catalog: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
    };

    let port = dashboard_port;
    tokio::spawn(async move {
        dashboard::serve(dash_state, port, "127.0.0.1").await;
    });

    // Swarm event loop
    let cmd_stream = UnboundedReceiverStream::new(cmd_rx);
    let c_cache = provider_cache.clone();
    let c_rep = rep.clone();
    let c_conn = conn.clone();
    let c_evt = event_tx.clone();
    let c_blocks = blocks.clone();
    let c_nar = nar_store.clone();
    let connect_addr = connect_addr.map(|s| s.parse::<Multiaddr>()).transpose()?;

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
            connect_addr.map(|a| vec![a]).unwrap_or_default(),
            block_size,
            "seed",
        )
        .await;
    });

    let listen_addr = addr_rx.await.context("node did not report address")?;

    println!();
    tracing::info!("seed node ready!");
    tracing::info!("  peer_id: {}", pid);
    tracing::info!("  listen:   {}", listen_addr);
    tracing::info!("  dashboard: http://127.0.0.1:{}", port);
    tracing::info!("  seeded {} hash(es)", seeder_hashes.len());
    for hash in &seeder_hashes {
        tracing::info!("    {}...", &hash[..16]);
    }
    println!();
    tracing::info!("other nodes can connect to download. press Ctrl-C to stop.");

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    Ok(())
}

// ── Real Guix smoke/benchmark harness ────────────────────────────────

struct ContainerSmokeOptions {
    package: String,
    store_path: Option<String>,
    transport: HarnessTransport,
    base: PathBuf,
    node_a_port: u16,
    node_b_port: u16,
    node_a_dashboard_port: u16,
    node_b_dashboard_port: u16,
    dashboard_bind: String,
    guix_p2p_bin: Option<PathBuf>,
    vm_direct: bool,
    hold: bool,
    keep_temp: bool,
}

struct BenchmarkOptions {
    packages: Vec<String>,
    modes: Vec<BenchmarkMode>,
    iterations: usize,
    transport: HarnessTransport,
    base: PathBuf,
    guix_p2p_bin: Option<PathBuf>,
    keep_temp: bool,
}

struct HarnessTools {
    guix: PathBuf,
    real_guix: PathBuf,
    raw_guix_daemon: PathBuf,
    guix_p2p: PathBuf,
    shell: PathBuf,
}

struct P2pBuildSpec<'a> {
    base: &'a std::path::Path,
    package: &'a str,
    store_path: &'a str,
    nar_hash: &'a str,
    transport: HarnessTransport,
    node_a_port: u16,
    node_b_port: u16,
    node_a_dashboard_port: u16,
    node_b_dashboard_port: u16,
    dashboard_bind: &'a str,
    node_b_policy: &'a str,
    strict_p2p_evidence: bool,
    hold_after_success: bool,
    vm_direct: bool,
    tools: &'a HarnessTools,
}

struct P2pBuildOutcome {
    elapsed_ms: u128,
    p2p_evidence: bool,
    nar_size: Option<u64>,
}

struct BenchmarkPackage {
    name: String,
    store_path: String,
    nar_hash: String,
}

struct BenchmarkRecord {
    package: String,
    store_path: String,
    nar_hash: String,
    nar_size: Option<u64>,
    mode: BenchmarkMode,
    iteration: usize,
    elapsed_ms: Option<u128>,
    success: bool,
    p2p_evidence: bool,
    run_dir: PathBuf,
    error: Option<String>,
}

struct ManagedChild {
    label: String,
    child: std::process::Child,
}

#[derive(Default)]
struct ProcessSet {
    children: Vec<ManagedChild>,
}

impl ProcessSet {
    fn spawn_logged(
        &mut self,
        label: &str,
        command: &mut std::process::Command,
        log_path: &std::path::Path,
    ) -> anyhow::Result<()> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .with_context(|| format!("failed to open log {}", log_path.display()))?;
        let stderr = log.try_clone().context("failed to clone log file")?;
        tracing::debug!("spawning {label}: {:?}", command);
        let child = command
            .stdout(std::process::Stdio::from(log))
            .stderr(std::process::Stdio::from(stderr))
            .spawn()
            .with_context(|| format!("failed to spawn {label}"))?;
        self.children.push(ManagedChild { label: label.to_string(), child });
        Ok(())
    }
}

impl Drop for ProcessSet {
    fn drop(&mut self) {
        for managed in &mut self.children {
            match managed.child.try_wait() {
                Ok(Some(_)) => {},
                Ok(None) => {
                    let _ = managed.child.kill();
                    let _ = managed.child.wait();
                },
                Err(e) => {
                    tracing::debug!("failed to inspect child {}: {}", managed.label, e);
                },
            }
        }
    }
}

async fn run_container_smoke(opts: ContainerSmokeOptions) -> anyhow::Result<()> {
    let tools = prepare_harness_tools(opts.guix_p2p_bin.as_deref())?;
    let base = absolutize_path(&project_root(), &opts.base);
    reset_dir(&base)?;
    std::fs::create_dir_all(base.join("logs"))?;
    ensure_container_guix_store_writable(&tools, &base, opts.vm_direct)?;

    let store_path = match opts.store_path {
        Some(path) => {
            if !std::path::Path::new(&path).exists() {
                anyhow::bail!("--store-path does not exist: {path}");
            }
            path
        },
        None => {
            tracing::info!("resolving Guix package {}", opts.package);
            resolve_package(&tools.guix, &opts.package)?
        },
    };
    let nar_hash = compute_nar_hash(&tools.guix, &store_path)?;

    tracing::info!("seed store path: {}", store_path);
    tracing::info!("nar hash: {}", nar_hash);

    let outcome = run_p2p_build(P2pBuildSpec {
        base: &base,
        package: &opts.package,
        store_path: &store_path,
        nar_hash: &nar_hash,
        transport: opts.transport,
        node_a_port: opts.node_a_port,
        node_b_port: opts.node_b_port,
        node_a_dashboard_port: opts.node_a_dashboard_port,
        node_b_dashboard_port: opts.node_b_dashboard_port,
        dashboard_bind: &opts.dashboard_bind,
        node_b_policy: "p2p-only",
        strict_p2p_evidence: true,
        hold_after_success: opts.hold,
        vm_direct: opts.vm_direct,
        tools: &tools,
    })
    .await?;

    tracing::info!(
        "container smoke passed: package={} elapsed={}ms p2p_evidence={} logs={}",
        opts.package,
        outcome.elapsed_ms,
        outcome.p2p_evidence,
        base.join("logs").display()
    );
    tracing::info!("seed store path: {}", store_path);
    tracing::info!("nar hash: {}", nar_hash);
    tracing::info!(
        "node A dashboard: http://{}:{}",
        opts.dashboard_bind,
        opts.node_a_dashboard_port
    );
    tracing::info!(
        "node B dashboard: http://{}:{}",
        opts.dashboard_bind,
        opts.node_b_dashboard_port
    );
    if opts.keep_temp {
        tracing::info!("kept generated state under {}", base.display());
    }
    Ok(())
}

async fn run_benchmark(opts: BenchmarkOptions) -> anyhow::Result<()> {
    if opts.iterations == 0 {
        anyhow::bail!("--iterations must be greater than zero");
    }
    if opts.packages.is_empty() {
        anyhow::bail!("--packages must contain at least one package");
    }
    if opts.modes.is_empty() {
        anyhow::bail!("--modes must contain at least one mode");
    }

    let tools = prepare_harness_tools(opts.guix_p2p_bin.as_deref())?;
    let base = absolutize_path(&project_root(), &opts.base);
    std::fs::create_dir_all(&base)?;
    let tmp_root = base.join("tmp");
    reset_dir(&tmp_root)?;
    ensure_container_guix_store_writable(&tools, &tmp_root, false)?;

    let mut packages = Vec::new();
    for package in &opts.packages {
        tracing::info!("resolving benchmark package {}", package);
        let store_path = resolve_package(&tools.guix, package)?;
        let nar_hash = compute_nar_hash(&tools.guix, &store_path)?;
        packages.push(BenchmarkPackage { name: package.clone(), store_path, nar_hash });
    }

    let mut records = Vec::new();
    let mut failures = Vec::new();
    let mut port_seed = 41_000_u16;
    let mut dash_seed = 31_000_u16;

    for package in &packages {
        for mode in &opts.modes {
            for iteration in 1..=opts.iterations {
                let run_dir = tmp_root.join(format!(
                    "{}-{}-{}",
                    sanitize_name(&package.name),
                    mode,
                    iteration
                ));
                reset_dir(&run_dir)?;
                std::fs::create_dir_all(run_dir.join("logs"))?;

                tracing::info!(
                    "benchmark package={} mode={} iteration={}/{}",
                    package.name,
                    mode,
                    iteration,
                    opts.iterations
                );

                let result = match mode {
                    BenchmarkMode::Http => run_http_benchmark(&run_dir, &package.name, &tools)
                        .map(|elapsed_ms| (elapsed_ms, false, None)),
                    BenchmarkMode::P2pOnly | BenchmarkMode::P2pFirst => {
                        let node_a_port = reserve_transport_port(opts.transport, &mut port_seed)?;
                        let node_b_port = reserve_transport_port(opts.transport, &mut port_seed)?;
                        let node_a_dashboard_port = reserve_tcp_port(&mut dash_seed)?;
                        let node_b_dashboard_port = reserve_tcp_port(&mut dash_seed)?;
                        let policy = mode.as_policy().expect("p2p mode has a policy");
                        run_p2p_build(P2pBuildSpec {
                            base: &run_dir,
                            package: &package.name,
                            store_path: &package.store_path,
                            nar_hash: &package.nar_hash,
                            transport: opts.transport,
                            node_a_port,
                            node_b_port,
                            node_a_dashboard_port,
                            node_b_dashboard_port,
                            dashboard_bind: "127.0.0.1",
                            node_b_policy: policy,
                            strict_p2p_evidence: *mode == BenchmarkMode::P2pOnly,
                            hold_after_success: false,
                            vm_direct: false,
                            tools: &tools,
                        })
                        .await
                        .map(|outcome| (outcome.elapsed_ms, outcome.p2p_evidence, outcome.nar_size))
                    },
                };

                match result {
                    Ok((elapsed_ms, p2p_evidence, nar_size)) => records.push(BenchmarkRecord {
                        package: package.name.clone(),
                        store_path: package.store_path.clone(),
                        nar_hash: package.nar_hash.clone(),
                        nar_size,
                        mode: *mode,
                        iteration,
                        elapsed_ms: Some(elapsed_ms),
                        success: true,
                        p2p_evidence,
                        run_dir: run_dir.clone(),
                        error: None,
                    }),
                    Err(e) => {
                        let message = e.to_string();
                        failures.push(format!(
                            "{} {} iteration {}: {}",
                            package.name, mode, iteration, message
                        ));
                        records.push(BenchmarkRecord {
                            package: package.name.clone(),
                            store_path: package.store_path.clone(),
                            nar_hash: package.nar_hash.clone(),
                            nar_size: None,
                            mode: *mode,
                            iteration,
                            elapsed_ms: None,
                            success: false,
                            p2p_evidence: false,
                            run_dir: run_dir.clone(),
                            error: Some(message),
                        });
                    },
                }

                if !opts.keep_temp {
                    let _ = std::fs::remove_dir_all(&run_dir);
                }
            }
        }
    }

    let csv_path = base.join("results.csv");
    write_benchmark_csv(&csv_path, &records)?;
    write_benchmark_report(
        &project_root().join("docs/benchmark-results.md"),
        &records,
        &packages,
        opts.transport,
        opts.iterations,
    )?;

    tracing::info!("benchmark CSV: {}", csv_path.display());
    tracing::info!("benchmark report: docs/benchmark-results.md");

    if !opts.keep_temp {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }

    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "benchmark completed with {} failed run(s); see docs/benchmark-results.md",
            failures.len()
        );
    }
}

fn prepare_harness_tools(guix_p2p_bin: Option<&std::path::Path>) -> anyhow::Result<HarnessTools> {
    let guix = find_on_path("guix").context("missing required command: guix")?;
    let shell = find_on_path("sh")
        .or_else(|| {
            canonicalize_existing(std::path::Path::new("/run/current-system/profile/bin/sh")).ok()
        })
        .context("missing required command: sh")?;
    let real_guix = canonicalize_existing(&guix)?;
    let raw_guix_daemon = resolve_raw_guix_daemon()?;
    let guix_p2p = match guix_p2p_bin {
        Some(path) => canonicalize_existing(path)?,
        None => {
            let cargo = find_on_path("cargo").context("missing required command: cargo")?;
            build_release_binary(&cargo)?;
            project_root().join("target/release/guix-p2p")
        },
    };
    if !guix_p2p.is_file() {
        anyhow::bail!("guix-p2p binary not found at {}", guix_p2p.display());
    }
    Ok(HarnessTools { guix, real_guix, raw_guix_daemon, guix_p2p, shell })
}

fn build_release_binary(cargo: &std::path::Path) -> anyhow::Result<()> {
    let mut command = std::process::Command::new(cargo);
    command.current_dir(project_root()).args(["build", "--release"]);
    checked_status(command, "cargo build --release")
}

fn guix_container_command(
    tools: &HarnessTools,
    base: &std::path::Path,
    vm_direct: bool,
) -> std::process::Command {
    if vm_direct || std::env::var_os("GUIX_P2P_E2E_NO_GUIX_SHELL").is_some() {
        if let Some(env) = find_on_path("env") {
            return std::process::Command::new(env);
        }
        let profile_env = std::path::Path::new("/run/current-system/profile/bin/env");
        if profile_env.exists() {
            return std::process::Command::new(profile_env);
        }
        return std::process::Command::new("env");
    }

    let mut command = std::process::Command::new(&tools.guix);
    command
        .arg("shell")
        .arg("-C")
        .arg("-N")
        .arg("--writable-root")
        .arg(format!("--share={}", base.display()));

    let root = project_root();
    if root.exists() {
        command.arg(format!("--share={}", root.display()));
    }

    if std::path::Path::new("/etc/guix").exists() {
        command.arg("--expose=/etc/guix");
    }
    if std::path::Path::new("/var/guix").exists() {
        command.arg("--expose=/var/guix");
    }

    command.arg("--");
    command
}

fn ensure_container_guix_store_writable(
    tools: &HarnessTools,
    base: &std::path::Path,
    vm_direct: bool,
) -> anyhow::Result<()> {
    if vm_direct || std::env::var_os("GUIX_P2P_E2E_NO_GUIX_SHELL").is_some() {
        let probe = std::path::Path::new("/gnu/store/.guix-p2p-e2e-write-test");
        std::fs::write(probe, b"probe").map_err(|e| {
            anyhow::anyhow!(
                "Guix VM E2E requires /gnu/store to be writable. The direct preflight failed: {e}"
            )
        })?;
        std::fs::remove_file(probe).ok();
        return Ok(());
    }

    let mut command = guix_container_command(tools, base, vm_direct);
    command.args([
        "/bin/sh",
        "-c",
        "test -w /gnu/store || { echo '/gnu/store is not writable inside guix shell -CN' >&2; \
         exit 1; }",
    ]);
    checked_status(command, "guix shell -CN writable /gnu/store preflight").map_err(|e| {
        anyhow::anyhow!(
            "Guix container E2E requires /gnu/store to be writable inside `guix shell -CN`. The \
             container preflight failed: {e}"
        )
    })
}

async fn run_p2p_build(spec: P2pBuildSpec<'_>) -> anyhow::Result<P2pBuildOutcome> {
    let logs_dir = spec.base.join("logs");
    std::fs::create_dir_all(&logs_dir)?;

    let node_a_dir = spec.base.join("node-a");
    let node_b_dir = spec.base.join("node-b");
    let node_a_cache = node_a_dir.join("cache");
    let node_b_cache = node_b_dir.join("cache");
    let node_a_config_home = node_a_dir.join("config-home");
    let node_b_config_home = node_b_dir.join("config-home");
    let node_a_socket = node_a_dir.join("guix-p2p.sock");
    let node_b_socket = node_b_dir.join("guix-p2p.sock");
    let daemon_socket = node_b_dir.join("guix-daemon.sock");
    let wrapper_path = node_b_dir.join("guix-wrapper.sh");

    std::fs::create_dir_all(&node_a_cache)?;
    std::fs::create_dir_all(&node_b_cache)?;
    std::fs::create_dir_all(&node_a_config_home)?;
    std::fs::create_dir_all(&node_b_config_home)?;

    let node_a_addr = spec.transport.listen_addr(spec.node_a_port);
    let node_b_addr = spec.transport.listen_addr(spec.node_b_port);

    write_node_config(NodeConfigSpec {
        xdg_config_home: &node_a_config_home,
        listen_addr: &node_a_addr,
        cache_dir: &node_a_cache,
        socket_path: &node_a_socket,
        substitute_policy: "p2p-only",
        min_providers: 1,
        dashboard_port: spec.node_a_dashboard_port,
        dashboard_bind: spec.dashboard_bind,
        bootstrap_peers: None,
        seed_paths: &[spec.store_path],
    })?;
    write_node_config(NodeConfigSpec {
        xdg_config_home: &node_b_config_home,
        listen_addr: &node_b_addr,
        cache_dir: &node_b_cache,
        socket_path: &node_b_socket,
        substitute_policy: spec.node_b_policy,
        min_providers: 1,
        dashboard_port: spec.node_b_dashboard_port,
        dashboard_bind: spec.dashboard_bind,
        bootstrap_peers: None,
        seed_paths: &[],
    })?;

    write_wrapper(
        &wrapper_path,
        &node_b_socket,
        &spec.tools.guix_p2p,
        &spec.tools.real_guix,
        &spec.tools.shell,
    )?;

    let mut processes = ProcessSet::default();

    let mut node_a_cmd = guix_container_command(spec.tools, spec.base, spec.vm_direct);
    node_a_cmd
        .arg(&spec.tools.guix_p2p)
        .arg("--daemon")
        .arg("--listen-addr")
        .arg(&node_a_addr)
        .arg("--cache-dir")
        .arg(&node_a_cache)
        .arg("--socket")
        .arg(&node_a_socket)
        .arg("--dashboard")
        .arg("--dashboard-port")
        .arg(spec.node_a_dashboard_port.to_string())
        .arg("--dashboard-bind")
        .arg(spec.dashboard_bind)
        .arg("--policy")
        .arg("p2p-only")
        .arg("--seed")
        .arg(spec.store_path)
        .env("HOME", &node_a_dir)
        .env("XDG_CONFIG_HOME", &node_a_config_home)
        .env("RUST_LOG", "guix_p2p=trace,info");
    let node_a_log = logs_dir.join("node-a.log");
    processes.spawn_logged("node-a", &mut node_a_cmd, &node_a_log)?;
    wait_dashboard(spec.node_a_dashboard_port, "node A", Some(&node_a_log))?;

    let node_a_status = dashboard_json(spec.node_a_dashboard_port, "/api/status")?;
    let node_a_peer = json_string(&node_a_status, "peer_id")
        .context("node A dashboard did not expose peer_id")?;
    let bootstrap = format!("{node_a_addr}/p2p/{node_a_peer}");

    maybe_remove_seed_store_path(spec.store_path)?;

    write_node_config(NodeConfigSpec {
        xdg_config_home: &node_b_config_home,
        listen_addr: &node_b_addr,
        cache_dir: &node_b_cache,
        socket_path: &node_b_socket,
        substitute_policy: spec.node_b_policy,
        min_providers: 1,
        dashboard_port: spec.node_b_dashboard_port,
        dashboard_bind: spec.dashboard_bind,
        bootstrap_peers: Some(&bootstrap),
        seed_paths: &[],
    })?;

    let mut node_b_cmd = guix_container_command(spec.tools, spec.base, spec.vm_direct);
    node_b_cmd
        .arg(&spec.tools.guix_p2p)
        .arg("--daemon")
        .arg("--listen-addr")
        .arg(&node_b_addr)
        .arg("--cache-dir")
        .arg(&node_b_cache)
        .arg("--socket")
        .arg(&node_b_socket)
        .arg("--dashboard")
        .arg("--dashboard-port")
        .arg(spec.node_b_dashboard_port.to_string())
        .arg("--dashboard-bind")
        .arg(spec.dashboard_bind)
        .arg("--policy")
        .arg(spec.node_b_policy)
        .arg("--bootstrap-peers")
        .arg(&bootstrap)
        .env("HOME", &node_b_dir)
        .env("XDG_CONFIG_HOME", &node_b_config_home)
        .env("RUST_LOG", "guix_p2p=trace,info");
    let node_b_log = logs_dir.join("node-b.log");
    processes.spawn_logged("node-b", &mut node_b_cmd, &node_b_log)?;
    wait_dashboard(spec.node_b_dashboard_port, "node B", Some(&node_b_log))?;
    wait_unix_socket(&node_b_socket, "node B relay socket")?;

    let guix_state = prepare_guix_daemon_state(&node_b_dir)?;
    let mut daemon_cmd = guix_container_command(spec.tools, spec.base, spec.vm_direct);
    daemon_cmd
        .arg(&spec.tools.raw_guix_daemon)
        .arg("--disable-chroot")
        .arg("--max-jobs=0")
        .arg(format!("--listen={}", daemon_socket.display()))
        .env("HOME", &node_b_dir)
        .env("GUIX", &wrapper_path)
        .env("GUIX_STATE_DIRECTORY", &guix_state.state_dir)
        .env("GUIX_CONFIGURATION_DIRECTORY", &guix_state.config_dir);
    processes.spawn_logged("guix-daemon", &mut daemon_cmd, &logs_dir.join("guix-daemon.log"))?;
    wait_unix_socket(&daemon_socket, "isolated guix-daemon socket")?;

    let elapsed_ms = run_guix_build_logged(
        spec.tools,
        spec.base,
        spec.package,
        &daemon_socket,
        &logs_dir.join("build.log"),
        spec.vm_direct,
    )?;

    let seeds = dashboard_json(spec.node_a_dashboard_port, "/api/seeds")?;
    let nar_size = validate_seeds(&seeds, spec.nar_hash)?;
    let _catalog = wait_for_catalog_entry(
        spec.node_b_dashboard_port,
        spec.store_path,
        spec.nar_hash,
        std::time::Duration::from_secs(30),
    )?;

    let node_a_log = std::fs::read_to_string(logs_dir.join("node-a.log")).unwrap_or_default();
    let node_b_log = std::fs::read_to_string(logs_dir.join("node-b.log")).unwrap_or_default();
    let p2p_evidence =
        contains_any(&node_a_log, &["Served block request", "serving ", "BlockServed"]);
    let node_b_success = contains_any(
        &node_b_log,
        &["Substitute download succeeded", "DownloadSucceeded", "download-succeeded"],
    );

    if spec.strict_p2p_evidence && !p2p_evidence {
        anyhow::bail!(
            "node A log did not show block serving evidence; see {}",
            logs_dir.join("node-a.log").display()
        );
    }
    if !node_b_success {
        anyhow::bail!(
            "node B log did not show a successful substitute download; see {}",
            logs_dir.join("node-b.log").display()
        );
    }
    if spec.node_b_policy == "p2p-only" {
        if !contains_any(&node_b_log, &["p2p-only", "P2pOnly"]) {
            anyhow::bail!(
                "node B log did not show p2p-only substitute handling; see {}",
                logs_dir.join("node-b.log").display()
            );
        }
        if contains_any(&node_b_log, &["falling back to HTTP", "Attempting HTTP nar download"]) {
            anyhow::bail!(
                "node B used an HTTP nar fallback in p2p-only mode; see {}",
                logs_dir.join("node-b.log").display()
            );
        }
    }

    if spec.hold_after_success {
        tracing::info!("strict P2P smoke proof passed; holding daemons until Ctrl-C");
        tracing::info!(
            "node A dashboard: http://{}:{}",
            spec.dashboard_bind,
            spec.node_a_dashboard_port
        );
        tracing::info!(
            "node B dashboard: http://{}:{}",
            spec.dashboard_bind,
            spec.node_b_dashboard_port
        );
        tokio::signal::ctrl_c().await.context("failed to wait for Ctrl-C")?;
        tracing::info!("Ctrl-C received; stopping smoke daemons");
    }

    drop(processes);

    Ok(P2pBuildOutcome { elapsed_ms, p2p_evidence, nar_size })
}

fn maybe_remove_seed_store_path(store_path: &str) -> anyhow::Result<()> {
    if std::env::var_os("GUIX_P2P_E2E_REMOVE_SEED_AFTER_NODE_A").is_none() {
        return Ok(());
    }
    let path = std::path::Path::new(store_path);
    if !path.starts_with("/gnu/store") || !path.exists() {
        return Ok(());
    }
    tracing::info!("removing seeded store path before builder run: {}", store_path);
    if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
        .with_context(|| format!("failed to remove seeded store path {store_path}"))
}

fn run_http_benchmark(
    run_dir: &std::path::Path,
    package: &str,
    tools: &HarnessTools,
) -> anyhow::Result<u128> {
    let logs_dir = run_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)?;
    let daemon_socket = run_dir.join("guix-daemon.sock");
    let guix_state = prepare_guix_daemon_state(run_dir)?;
    let mut processes = ProcessSet::default();

    let mut daemon_cmd = guix_container_command(tools, run_dir, false);
    daemon_cmd
        .arg(&tools.raw_guix_daemon)
        .arg("--disable-chroot")
        .arg("--max-jobs=0")
        .arg(format!("--listen={}", daemon_socket.display()))
        .env("HOME", run_dir)
        .env("GUIX", &tools.real_guix)
        .env("GUIX_STATE_DIRECTORY", &guix_state.state_dir)
        .env("GUIX_CONFIGURATION_DIRECTORY", &guix_state.config_dir);
    processes.spawn_logged("guix-daemon", &mut daemon_cmd, &logs_dir.join("guix-daemon.log"))?;
    wait_unix_socket(&daemon_socket, "isolated guix-daemon socket")?;
    let elapsed_ms = run_guix_build_logged(
        tools,
        run_dir,
        package,
        &daemon_socket,
        &logs_dir.join("build.log"),
        false,
    )?;
    drop(processes);
    Ok(elapsed_ms)
}

struct NodeConfigSpec<'a> {
    xdg_config_home: &'a std::path::Path,
    listen_addr: &'a str,
    cache_dir: &'a std::path::Path,
    socket_path: &'a std::path::Path,
    substitute_policy: &'a str,
    min_providers: usize,
    dashboard_port: u16,
    dashboard_bind: &'a str,
    bootstrap_peers: Option<&'a str>,
    seed_paths: &'a [&'a str],
}

fn write_node_config(spec: NodeConfigSpec<'_>) -> anyhow::Result<()> {
    let config_dir = spec.xdg_config_home.join("guix-p2p");
    std::fs::create_dir_all(&config_dir)?;
    let mut toml = String::new();
    toml.push_str(&format!("listen_addr = {}\n", toml_string(spec.listen_addr)));
    toml.push_str(&format!("cache_dir = {}\n", toml_string(&spec.cache_dir.display().to_string())));
    toml.push_str(&format!(
        "socket_path = {}\n",
        toml_string(&spec.socket_path.display().to_string())
    ));
    toml.push_str(&format!("substitute_policy = {}\n", toml_string(spec.substitute_policy)));
    toml.push_str(&format!("min_providers = {}\n", spec.min_providers));
    toml.push_str("request_timeout_secs = 60\n");
    toml.push_str("stall_timeout_secs = 30\n");
    toml.push_str("dashboard_enabled = true\n");
    toml.push_str(&format!("dashboard_port = {}\n", spec.dashboard_port));
    toml.push_str(&format!("dashboard_bind = {}\n", toml_string(spec.dashboard_bind)));
    toml.push_str("substitute_urls = \"https://bordeaux.guix.gnu.org,https://ci.guix.gnu.org\"\n");
    if let Some(peers) = spec.bootstrap_peers {
        toml.push_str(&format!("bootstrap_peers = {}\n", toml_string(peers)));
    }
    toml.push_str("seed_paths = [");
    for (idx, path) in spec.seed_paths.iter().enumerate() {
        if idx > 0 {
            toml.push_str(", ");
        }
        toml.push_str(&toml_string(path));
    }
    toml.push_str("]\n");
    std::fs::write(config_dir.join("config.toml"), toml)?;
    Ok(())
}

struct GuixDaemonState {
    state_dir: PathBuf,
    config_dir: PathBuf,
}

fn prepare_guix_daemon_state(base: &std::path::Path) -> anyhow::Result<GuixDaemonState> {
    let state_dir = base.join("state");
    let config_dir = base.join("etc");
    for rel in ["db", "daemon-socket", "gcroots", "profiles", "substitute", "temproots", "userpool"]
    {
        std::fs::create_dir_all(state_dir.join(rel))?;
    }
    std::fs::create_dir_all(&config_dir)?;
    let host_acl = std::path::Path::new("/etc/guix/acl");
    if host_acl.exists() {
        let _ = std::fs::copy(host_acl, config_dir.join("acl"));
    }
    Ok(GuixDaemonState { state_dir, config_dir })
}

fn write_wrapper(
    wrapper: &std::path::Path,
    socket: &std::path::Path,
    guix_p2p: &std::path::Path,
    real_guix: &std::path::Path,
    shell: &std::path::Path,
) -> anyhow::Result<()> {
    let content = format!(
        r#"#!{}
SOCKET={}
GUIX_P2P={}
REAL_GUIX={}

case "${{1-}}" in
    substitute)
        shift
        case "${{1-}}" in
            --query|--substitute)
                if [ -S "$SOCKET" ]; then
                    exec "$GUIX_P2P" "$@" --socket "$SOCKET"
                fi
                exec "$REAL_GUIX" substitute "$@"
                ;;
            *)
                exec "$REAL_GUIX" substitute "$@"
                ;;
        esac
        ;;
    *)
        exec "$REAL_GUIX" "$@"
        ;;
esac
"#,
        shell.display(),
        shell_quote(&socket.display().to_string()),
        shell_quote(&guix_p2p.display().to_string()),
        shell_quote(&real_guix.display().to_string())
    );
    std::fs::write(wrapper, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(wrapper)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(wrapper, permissions)?;
    }
    Ok(())
}

fn run_guix_build_logged(
    tools: &HarnessTools,
    base: &std::path::Path,
    package: &str,
    daemon_socket: &std::path::Path,
    log_path: &std::path::Path,
    vm_direct: bool,
) -> anyhow::Result<u128> {
    let log = std::fs::OpenOptions::new().create(true).append(true).open(log_path)?;
    let stderr = log.try_clone()?;
    let started = std::time::Instant::now();
    let mut command = guix_container_command(tools, base, vm_direct);
    command.arg(&tools.guix).arg("build").arg(package).env("GUIX_DAEMON_SOCKET", daemon_socket);
    tracing::debug!("running build command: {:?}", command);
    let status = command
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(stderr))
        .status()
        .with_context(|| format!("failed to run guix build {package}"))?;
    let elapsed_ms = started.elapsed().as_millis();
    if !status.success() {
        anyhow::bail!(
            "guix build {} failed with {}; log tail:\n{}",
            package,
            status,
            read_tail(log_path, 80)
        );
    }
    Ok(elapsed_ms)
}

fn wait_dashboard(
    port: u16,
    label: &str,
    log_path: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        match dashboard_json(port, "/api/status") {
            Ok(_) => return Ok(()),
            Err(e) if std::time::Instant::now() < deadline => {
                tracing::debug!("waiting for {label} dashboard: {e}");
                std::thread::sleep(std::time::Duration::from_millis(500));
            },
            Err(e) => {
                let tail = log_path.map(|path| read_tail(path, 80)).unwrap_or_default();
                anyhow::bail!(
                    "timed out waiting for {label} dashboard on {port}: {e}\n{} log tail:\n{}",
                    label,
                    tail
                );
            },
        }
    }
}

fn wait_unix_socket(path: &std::path::Path, label: &str) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if is_unix_socket(path) {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for {label} at {}", path.display());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

fn dashboard_json(port: u16, path: &str) -> anyhow::Result<serde_json::Value> {
    let body = http_get_body(port, path)?;
    serde_json::from_str(&body).with_context(|| format!("dashboard returned invalid JSON: {body}"))
}

fn http_get_body(port: u16, path: &str) -> anyhow::Result<String> {
    use std::io::{Read, Write};

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream =
        std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    if !response.starts_with("HTTP/1.1 200") && !response.starts_with("HTTP/1.0 200") {
        anyhow::bail!("dashboard HTTP response was not 200: {}", first_line(&response));
    }
    let (_, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("dashboard response did not contain a body"))?;
    Ok(body.to_string())
}

fn wait_for_catalog_entry(
    dashboard_port: u16,
    store_path: &str,
    nar_hash: &str,
    timeout: std::time::Duration,
) -> anyhow::Result<serde_json::Value> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let catalog = dashboard_json(dashboard_port, "/api/catalog")?;
        if catalog_entry_matches(&catalog, store_path, nar_hash) {
            return Ok(catalog);
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!(
                "node B catalog did not include {} or nar hash {}; last catalog: {}",
                store_path,
                nar_hash,
                catalog
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

fn validate_seeds(seeds: &serde_json::Value, nar_hash: &str) -> anyhow::Result<Option<u64>> {
    let entries =
        seeds.as_array().ok_or_else(|| anyhow::anyhow!("/api/seeds did not return an array"))?;
    if entries.is_empty() {
        anyhow::bail!("node A /api/seeds was empty");
    }
    for entry in entries {
        if entry.get("nar_hash").and_then(serde_json::Value::as_str) == Some(nar_hash) {
            return Ok(entry.get("nar_size").and_then(serde_json::Value::as_u64));
        }
    }
    anyhow::bail!("node A /api/seeds did not include seeded nar {}", nar_hash);
}

fn catalog_entry_matches(catalog: &serde_json::Value, store_path: &str, nar_hash: &str) -> bool {
    let Some(entries) = catalog.as_array() else {
        return false;
    };
    let prefixed_nar_hash = format!("sha256:{nar_hash}");
    let hash_part = store_hash_part(store_path);
    entries.iter().any(|entry| {
        entry.get("store_path").and_then(serde_json::Value::as_str) == Some(store_path)
            || entry.get("nar_hash").and_then(serde_json::Value::as_str) == Some(&prefixed_nar_hash)
            || hash_part.as_deref().is_some_and(|h| {
                entry.get("hash_part").and_then(serde_json::Value::as_str) == Some(h)
            })
    })
}

fn resolve_package(guix: &std::path::Path, package: &str) -> anyhow::Result<String> {
    let output = checked_output(
        std::process::Command::new(guix).arg("build").arg(package),
        &format!("guix build {package}"),
    )?;
    let stdout = String::from_utf8(output.stdout)?;
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("/gnu/store/"))
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("guix build {} did not print a store path", package))
}

fn compute_nar_hash(guix: &std::path::Path, store_path: &str) -> anyhow::Result<String> {
    let output = checked_output(
        std::process::Command::new(guix).args(["hash", "-S", "nar", "-f", "hex", store_path]),
        &format!("guix hash -S nar -f hex {store_path}"),
    )?;
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn checked_status(mut command: std::process::Command, description: &str) -> anyhow::Result<()> {
    let output = command.output().with_context(|| format!("failed to run {description}"))?;
    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "{} failed with {}\nstdout:\n{}\nstderr:\n{}",
            description,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn checked_output(
    command: &mut std::process::Command,
    description: &str,
) -> anyhow::Result<std::process::Output> {
    let output = command.output().with_context(|| format!("failed to run {description}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        anyhow::bail!(
            "{} failed with {}\nstdout:\n{}\nstderr:\n{}",
            description,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn resolve_raw_guix_daemon() -> anyhow::Result<PathBuf> {
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir("/gnu/store").context("failed to read /gnu/store")? {
        let entry = entry?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if !name.contains("-guix-") {
            continue;
        }
        let candidate = entry.path().join("bin/guix-daemon");
        if candidate.is_file() && is_elf(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("could not find raw ELF guix-daemon under /gnu/store"))
}

fn is_elf(path: &std::path::Path) -> bool {
    use std::io::Read;

    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic).is_ok() && magic == *b"\x7fELF"
}

fn is_unix_socket(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        std::fs::metadata(path).map(|m| m.file_type().is_socket()).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

fn reserve_transport_port(transport: HarnessTransport, next: &mut u16) -> anyhow::Result<u16> {
    match transport {
        HarnessTransport::Tcp => reserve_tcp_port(next),
        HarnessTransport::Quic => reserve_udp_port(next),
    }
}

fn reserve_tcp_port(next: &mut u16) -> anyhow::Result<u16> {
    while *next < 60_000 {
        let port = *next;
        *next += 1;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    anyhow::bail!("could not reserve a TCP port")
}

fn reserve_udp_port(next: &mut u16) -> anyhow::Result<u16> {
    while *next < 60_000 {
        let port = *next;
        *next += 1;
        if std::net::UdpSocket::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    anyhow::bail!("could not reserve a UDP port")
}

fn write_benchmark_csv(path: &std::path::Path, records: &[BenchmarkRecord]) -> anyhow::Result<()> {
    let mut csv = String::from(
        "package,store_path,nar_hash,nar_size,mode,iteration,elapsed_ms,success,p2p_evidence,\
         run_dir,error\n",
    );
    for record in records {
        csv.push_str(&csv_row(&[
            record.package.clone(),
            record.store_path.clone(),
            record.nar_hash.clone(),
            record.nar_size.map(|n| n.to_string()).unwrap_or_default(),
            record.mode.to_string(),
            record.iteration.to_string(),
            record.elapsed_ms.map(|n| n.to_string()).unwrap_or_default(),
            record.success.to_string(),
            record.p2p_evidence.to_string(),
            record.run_dir.display().to_string(),
            record.error.clone().unwrap_or_default(),
        ]));
        csv.push('\n');
    }
    std::fs::write(path, csv)?;
    Ok(())
}

fn write_benchmark_report(
    path: &std::path::Path,
    records: &[BenchmarkRecord],
    packages: &[BenchmarkPackage],
    transport: HarnessTransport,
    iterations: usize,
) -> anyhow::Result<()> {
    let mut report = String::new();
    report.push_str("# Benchmark Results\n\n");
    report.push_str("Generated by `guix-p2p-e2e benchmark`.\n\n");
    report.push_str("## Host\n\n");
    report.push_str(&format!("- Date: {}\n", unix_timestamp()));
    report.push_str(&format!("- Platform: {} {}\n", std::env::consts::OS, std::env::consts::ARCH));
    report.push_str(&format!("- Transport: {transport}\n"));
    report.push_str(&format!("- Iterations: {iterations}\n"));
    report.push_str(&format!("- Rust: {}\n\n", rust_version()));

    report.push_str("## Packages\n\n");
    report.push_str("| Package | Store path | Nar hash | Nar size |\n");
    report.push_str("|---------|------------|----------|----------|\n");
    for package in packages {
        let nar_size = records
            .iter()
            .find(|r| r.package == package.name && r.nar_size.is_some())
            .and_then(|r| r.nar_size)
            .map(format_bytes)
            .unwrap_or_else(|| "unknown".to_string());
        report.push_str(&format!(
            "| {} | `{}` | `{}` | {} |\n",
            package.name, package.store_path, package.nar_hash, nar_size
        ));
    }

    report.push_str("\n## Runs\n\n");
    report.push_str("| Package | Mode | Iteration | Elapsed | Success | P2P evidence |\n");
    report.push_str("|---------|------|-----------|---------|---------|--------------|\n");
    for record in records {
        let elapsed = record
            .elapsed_ms
            .map(|ms| format!("{:.3}s", ms as f64 / 1000.0))
            .unwrap_or_else(|| "failed".to_string());
        report.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            record.package,
            record.mode,
            record.iteration,
            elapsed,
            record.success,
            record.p2p_evidence
        ));
    }

    report.push_str("\n## Medians\n\n");
    report.push_str(
        "| Package | Mode | Median elapsed | Successful runs | P2P evidence observed |\n",
    );
    report.push_str(
        "|---------|------|----------------|-----------------|-----------------------|\n",
    );
    for package in packages {
        let mut modes: Vec<BenchmarkMode> =
            records.iter().filter(|r| r.package == package.name).map(|r| r.mode).collect();
        modes.sort_by_key(|m| m.to_string());
        modes.dedup();
        for mode in modes {
            let subset: Vec<&BenchmarkRecord> =
                records.iter().filter(|r| r.package == package.name && r.mode == mode).collect();
            let median = median_ms(subset.iter().filter_map(|r| r.elapsed_ms).collect());
            let successes = subset.iter().filter(|r| r.success).count();
            let p2p = subset.iter().any(|r| r.p2p_evidence);
            report.push_str(&format!(
                "| {} | {} | {} | {}/{} | {} |\n",
                package.name,
                mode,
                median
                    .map(|ms| format!("{:.3}s", ms as f64 / 1000.0))
                    .unwrap_or_else(|| "n/a".to_string()),
                successes,
                subset.len(),
                p2p
            ));
        }
    }

    let failures: Vec<&BenchmarkRecord> = records.iter().filter(|r| !r.success).collect();
    if !failures.is_empty() {
        report.push_str("\n## Failed Runs\n\n");
        for failure in failures {
            report.push_str(&format!(
                "- {} {} iteration {}: {}\n",
                failure.package,
                failure.mode,
                failure.iteration,
                failure.error.as_deref().unwrap_or("unknown error")
            ));
        }
    }

    std::fs::write(path, report)?;
    Ok(())
}

fn median_ms(mut values: Vec<u128>) -> Option<u128> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(values[values.len() / 2])
}

fn csv_row(fields: &[String]) -> String {
    fields.iter().map(|field| csv_field(field)).collect::<Vec<_>>().join(",")
}

fn csv_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

fn rust_version() -> String {
    let Some(rustc) = find_on_path("rustc") else {
        return "unknown".to_string();
    };
    let Ok(output) = std::process::Command::new(rustc).arg("--version").output() else {
        return "unknown".to_string();
    };
    if output.status.success() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        "unknown".to_string()
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn canonicalize_existing(path: &std::path::Path) -> anyhow::Result<PathBuf> {
    path.canonicalize().with_context(|| format!("failed to canonicalize {}", path.display()))
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("e2e crate has a parent workspace")
        .to_path_buf()
}

fn absolutize_path(root: &std::path::Path, path: &std::path::Path) -> PathBuf {
    if path.is_absolute() { path.to_path_buf() } else { root.join(path) }
}

fn reset_dir(path: &std::path::Path) -> anyhow::Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
    }
    std::fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))
}

fn store_hash_part(store_path: &str) -> Option<String> {
    let rest = store_path.strip_prefix("/gnu/store/")?;
    Some(rest.split_once('-')?.0.to_string())
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn json_string(value: &serde_json::Value, field: &str) -> Option<String> {
    value.get(field)?.as_str().map(str::to_string)
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn sanitize_name(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn first_line(value: &str) -> &str {
    value.lines().next().unwrap_or("")
}

fn read_tail(path: &std::path::Path, max_lines: usize) -> String {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

fn unix_timestamp() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => format!("{} seconds since 1970-01-01 UTC", duration.as_secs()),
        Err(_) => "unknown".to_string(),
    }
}

fn unix_timestamp_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.2} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.2} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.2} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}
