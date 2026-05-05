//! Interactive 2-node demo with a live web dashboard.
//!
//! Starts a seeder holding nar data, a downloader with an embedded dashboard,
//! connects them, and performs block exchanges. Open http://127.0.0.1:3031.
//!
//! ```sh
//! cargo run --manifest-path e2e/Cargo.toml --example demo
//! ```

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::Context;
use futures::StreamExt;
use guix_p2p_substitute::{
    behaviour::{GuixP2PBehaviour, GuixP2PEvent, create_swarm_behaviour},
    channel::SwarmCommand,
    connection::{ConnectionConfig, ConnectionManager},
    dashboard::{self, BuildRegistry, DashboardEvent, ObservedBuild},
    dht::{self, ProviderCache},
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

const DASHBOARD_PORT: u16 = 3031;
const BLOCK_SIZE: usize = 65536;

fn make_nar(seed: u8, size: usize) -> (Vec<u8>, String) {
    let mut data = vec![seed; size];
    for (i, b) in data.iter_mut().enumerate() {
        *b = seed.wrapping_add((i % 251) as u8);
    }
    let hash = hex::encode(sha2::Sha256::digest(&data));
    (data, hash)
}

type BlockMap = Arc<std::sync::Mutex<HashMap<String, Vec<u8>>>>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "guix_p2p_e2e=info,guix_p2p_substitute=info,info".into()),
        )
        .init();

    let (nar_data, nar_hash) = make_nar(0x42, 500_000);
    let nar_size = nar_data.len() as u64;
    let block_count = (nar_size as usize).div_ceil(BLOCK_SIZE) as u32;

    println!();
    tracing::info!("╔══════════════════════════════════════════╗");
    tracing::info!("║    guix-p2p-substitute  2-node demo     ║");
    tracing::info!("╠══════════════════════════════════════════╣");
    tracing::info!("║  nar hash: {:.24}…  ║", &nar_hash[..24]);
    tracing::info!("║  nar size: {:>5}  blocks: {:>3}          ║", nar_size, block_count);
    tracing::info!("╚══════════════════════════════════════════╝");
    println!();

    // ── Step 1: seeder node ─────────────────────────────────────────
    let seeded: HashMap<String, Vec<u8>> = [(nar_hash.clone(), nar_data)].into();
    let blocks: BlockMap = Arc::new(std::sync::Mutex::new(seeded));

    let skp = libp2p::identity::Keypair::generate_ed25519();
    let spid = PeerId::from(skp.public());
    let mut sswarm = build_swarm(&skp)?;
    sswarm.listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse()?)?;

    for h in blocks.lock().unwrap().keys() {
        sswarm.behaviour_mut().kad.start_providing(RecordKey::new(&hex::decode(h)?))?;
    }

    let (saddr_tx, saddr_rx) = oneshot::channel::<Multiaddr>();
    let sblocks = blocks.clone();
    tokio::spawn(async { run_seeder(sswarm, sblocks, saddr_tx).await });

    let seeder_addr = saddr_rx.await.context("seeder did not report listen address")?;
    tracing::info!("seeder  pid={}  addr={}", spid, seeder_addr);

    // ── Step 2: downloader / dashboard node ─────────────────────────
    let dkp = libp2p::identity::Keypair::generate_ed25519();
    let dpid = PeerId::from(dkp.public());
    let mut dswarm = build_swarm(&dkp)?;
    dswarm.listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse()?)?;

    let provider_cache = dht::create_provider_cache();
    let rep = Arc::new(std::sync::Mutex::new(ReputationTracker::new(5)));
    let conn = Arc::new(std::sync::Mutex::new(ConnectionManager::new(ConnectionConfig::default())));
    let build_reg: BuildRegistry = Arc::new(std::sync::Mutex::new(HashMap::new()));
    let (event_tx, _) = tokio::sync::broadcast::channel::<DashboardEvent>(256);
    let (daddr_tx, daddr_rx) = oneshot::channel::<Multiaddr>();
    let (cmd_tx, cmd_rx) = unbounded_channel::<SwarmCommand>();

    let cmd_stream = UnboundedReceiverStream::new(cmd_rx);

    let c_cache = provider_cache.clone();
    let c_rep = rep.clone();
    let c_conn = conn.clone();
    let c_evt = event_tx.clone();

    tokio::spawn(async move {
        run_downloader(dswarm, c_cache, c_rep, c_conn, c_evt, daddr_tx, seeder_addr, cmd_stream)
            .await;
    });

    let dash_addr = daddr_rx.await.context("download node did not report listen address")?;
    tracing::info!("dash    pid={}  addr={}", dpid, dash_addr);

    // Populate build registry
    build_reg.lock().unwrap().insert(
        nar_hash.clone(),
        ObservedBuild {
            nar_hash: nar_hash.clone(),
            store_path: Some(format!("/gnu/store/{}--demo-pkg-1.0", &nar_hash[..32])),
            nar_size: Some(nar_size),
            references: vec!["/gnu/store/abc-ref-lib".into()],
            deriver: Some("/gnu/store/abc-demo.drv".into()),
            narinfo_raw: Some("StorePath: /gnu/store/demo\nNarHash: …\nNarSize: 500000\n".into()),
            providers: vec![spid.to_string()],
            downloaded_at: None,
            download_size: None,
        },
    );

    // ── Step 3: dashboard HTTP server ───────────────────────────────
    let dash_state = dashboard::DashboardState {
        provider_cache: provider_cache.clone(),
        reputation: rep.clone(),
        conn_mgr: conn.clone(),
        build_registry: build_reg.clone(),
        started: std::time::Instant::now(),
        peer_id: dpid.to_string(),
        event_bus: event_tx.clone(),
    };

    tokio::spawn(async move {
        dashboard::serve(dash_state, DASHBOARD_PORT, "127.0.0.1").await;
    });

    tracing::info!("dashboard on http://127.0.0.1:{}", DASHBOARD_PORT,);

    // ── Step 4: wait for connection then exchange blocks ────────────
    tokio::time::sleep(Duration::from_secs(2)).await;

    let nar_bytes = hex::decode(&nar_hash).unwrap();

    // We don't know the seeder PeerId from within the downloader easily.
    // But we know `spid` from outside — send the request via the channel
    // with spid.
    tracing::info!("→ handshake request");
    let _ = cmd_tx.send(SwarmCommand::SendBlockRequest {
        peer: spid,
        request: BlockRequest::Handshake { nar_hash: nar_bytes.clone() },
    });

    tokio::time::sleep(Duration::from_secs(1)).await;

    tracing::info!("→ requesting {block_count} blocks");
    let _ = cmd_tx.send(SwarmCommand::SendBlockRequest {
        peer: spid,
        request: BlockRequest::GetBlocks { indices: (0u32..block_count).collect() },
    });

    tracing::info!("open http://127.0.0.1:{} ← live events", DASHBOARD_PORT);
    tracing::info!("press Ctrl-C to stop");

    tokio::signal::ctrl_c().await?;
    tracing::info!("bye");
    Ok(())
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

// ── seeder event loop ───────────────────────────────────────────────

async fn run_seeder(
    mut swarm: libp2p::Swarm<GuixP2PBehaviour>,
    blocks: BlockMap,
    addr_tx: oneshot::Sender<Multiaddr>,
) {
    let mut addr_opt = Some(addr_tx);
    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                if let Some(atx) = addr_opt.take() {
                    let full = address
                        .clone()
                        .with(libp2p::multiaddr::Protocol::P2p(*swarm.local_peer_id()));
                    let _ = atx.send(full);
                }
            },
            SwarmEvent::Behaviour(GuixP2PEvent::BlockExchange(
                request_response::Event::Message {
                    message: request_response::Message::Request { request, channel, .. },
                    ..
                },
            )) => {
                let resp = serve_blocks(&blocks, &request);
                let _ = swarm.behaviour_mut().block_exchange.send_response(channel, resp);
            },
            _ => {},
        }
    }
}

// ── downloader / dashboard event loop ───────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_downloader(
    mut swarm: libp2p::Swarm<GuixP2PBehaviour>,
    cache: ProviderCache,
    rep: Arc<std::sync::Mutex<ReputationTracker>>,
    conn: Arc<std::sync::Mutex<ConnectionManager>>,
    evt: dashboard::EventBus,
    addr_tx: oneshot::Sender<Multiaddr>,
    seeder: Multiaddr,
    mut cmds: UnboundedReceiverStream<SwarmCommand>,
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
                        let _ = swarm.dial(seeder.clone());
                    }
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    conn.lock().unwrap().on_connected(peer_id);
                    rep.lock().unwrap().record_success(peer_id, 0);
                    let _ = evt.send(DashboardEvent::PeerConnected {
                        peer_id: peer_id.to_string(),
                        addresses: vec![],
                    });
                    let _ = swarm.behaviour_mut().kad.bootstrap();
                    tracing::info!("connected to {}", peer_id);
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
                    let _ = evt.send(DashboardEvent::ProvidersFound {
                        nar_hash: kh,
                        provider_count: providers.len(),
                    });
                }
                SwarmEvent::Behaviour(GuixP2PEvent::BlockExchange(
                    request_response::Event::Message {
                        peer,
                        message: request_response::Message::Response { response, .. },
                        ..
                    },
                )) => {
                    handle_block_response(&peer, &response, &evt);
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
                    tracing::info!("sent block request → {}", peer);
                }
            },
        }
    }
}

fn handle_block_response(peer: &PeerId, resp: &BlockResponse, evt: &dashboard::EventBus) {
    match resp {
        BlockResponse::HandshakeReply { blocks_available, .. } => {
            tracing::info!(
                "← handshake reply from {}: {} blocks available",
                peer,
                blocks_available.len(),
            );
        },
        BlockResponse::Blocks { data } => {
            let cnt = data.len();
            let total: usize = data.iter().map(|b| b.data.len()).sum();
            tracing::info!(
                "← received {cnt} blocks ({:.1} KB) from {}",
                total as f64 / 1024.0,
                peer,
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
}

fn serve_blocks(blocks: &BlockMap, req: &BlockRequest) -> BlockResponse {
    match req {
        BlockRequest::Handshake { nar_hash } => {
            let key = hex::encode(nar_hash);
            if let Some(data) = blocks.lock().unwrap().get(&key) {
                let hashes = compute_block_hashes(data, BLOCK_SIZE);
                let n = hashes.len() as u32;
                BlockResponse::HandshakeReply {
                    blocks_available: (0..n).collect(),
                    block_count: n,
                    block_size: BLOCK_SIZE as u32,
                    block_hashes: hashes.iter().map(|h| h.to_vec()).collect(),
                }
            } else {
                BlockResponse::HandshakeReply {
                    blocks_available: vec![],
                    block_count: 0,
                    block_size: BLOCK_SIZE as u32,
                    block_hashes: vec![],
                }
            }
        },
        BlockRequest::GetBlocks { indices } => {
            if let Some(data) = blocks.lock().unwrap().values().next() {
                let blks: Vec<BlockData> = indices
                    .iter()
                    .filter_map(|&i| {
                        let off = i as usize * BLOCK_SIZE;
                        (off < data.len()).then(|| {
                            let end = (off + BLOCK_SIZE).min(data.len());
                            BlockData { index: i, data: data[off..end].to_vec() }
                        })
                    })
                    .collect();
                BlockResponse::Blocks { data: blks }
            } else {
                BlockResponse::Error { message: "no blocks seeded".into() }
            }
        },
    }
}
