use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Context;
use futures::StreamExt;
use guix_p2p_substitute::{
    behaviour::{GuixP2PBehaviour, GuixP2PEvent, create_swarm_behaviour},
    channel::{SwarmCommand, SwarmNotification},
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
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    oneshot,
};
use tokio_stream::wrappers::UnboundedReceiverStream;

pub const BLOCK_SIZE: usize = 65536;

type BlockMap = Arc<Mutex<HashMap<String, Vec<u8>>>>;

pub struct TestNode {
    pub peer_id: PeerId,
    listen_addr: Multiaddr,
    cmd_tx: UnboundedSender<SwarmCommand>,
    notify_rx: Option<UnboundedReceiver<SwarmNotification>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    _task: tokio::task::JoinHandle<()>,
}

impl Drop for TestNode {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.take().map(|tx| tx.send(()));
    }
}

impl TestNode {
    pub fn pid(&self) -> PeerId { self.peer_id }
    pub fn addr(&self) -> Multiaddr { self.listen_addr.clone() }

    pub fn send_req(&self, peer: PeerId, req: BlockRequest) {
        let _ = self.cmd_tx.send(SwarmCommand::SendBlockRequest { peer, request: req });
    }

    pub fn lookup(&self, nar_hash: &str) {
        let _ = self.cmd_tx.send(SwarmCommand::GetProviders { hash: nar_hash.into() });
    }

    async fn recv(&mut self, deadline: Duration) -> Option<SwarmNotification> {
        match &mut self.notify_rx {
            Some(rx) => tokio::time::timeout(deadline, rx.recv()).await.ok().flatten(),
            None => None,
        }
    }

    pub async fn wait_providers(&mut self, nar_hash: &str, timeout: Duration) -> Vec<PeerId> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let rem = deadline.saturating_duration_since(tokio::time::Instant::now());
            if rem.is_zero() {
                return vec![];
            }
            match self.recv(Duration::from_secs(3)).await {
                Some(SwarmNotification::ProvidersFound { hash, peers }) if hash == nar_hash => {
                    return peers;
                },
                None => return vec![],
                _ => {},
            }
        }
    }

    pub async fn wait_block_resp(
        &mut self,
        peer: PeerId,
        timeout: Duration,
    ) -> Option<BlockResponse> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let rem = deadline.saturating_duration_since(tokio::time::Instant::now());
            if rem.is_zero() { return None; }
            match self.recv(Duration::from_secs(2)).await {
                Some(SwarmNotification::BlockResponse { peer: p, response }) if p == peer => {
                    return Some(response);
                },
                None => return None,
                _ => {},
            }
        }
    }
}

impl TestNode {
    pub async fn seeder(nar_hash: &str, nar_data: Vec<u8>) -> anyhow::Result<TestNode> {
        let mut map = HashMap::new();
        map.insert(nar_hash.to_string(), nar_data);
        start(map, &[]).await
    }

    pub async fn node(seeder_addr: Multiaddr) -> anyhow::Result<TestNode> {
        start(HashMap::new(), &[seeder_addr]).await
    }
}

async fn start(
    seed: HashMap<String, Vec<u8>>,
    bootstrap: &[Multiaddr],
) -> anyhow::Result<TestNode> {
    let kp = libp2p::identity::Keypair::generate_ed25519();
    let pid = PeerId::from(kp.public());
    let mut swarm = build_swarm(&kp)?;

    let target: Multiaddr = "/ip4/127.0.0.1/udp/0/quic-v1".parse()?;
    swarm.listen_on(target)?;

    // Pre-seed Kademlia provider records before starting the loop
    let blocks: BlockMap = Arc::new(Mutex::new(seed));
    for hash in blocks.lock().unwrap().keys() {
        let bytes = hex::decode(hash).context("invalid nar hash hex")?;
        swarm.behaviour_mut().kad.start_providing(RecordKey::new(&bytes))?;
    }

    let (cmd_tx, cmd_rx) = unbounded_channel::<SwarmCommand>();
    let (notify_tx, notify_rx) = unbounded_channel::<SwarmNotification>();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (addr_tx, addr_rx) = oneshot::channel::<Multiaddr>();

    let cmd_stream = UnboundedReceiverStream::new(cmd_rx);
    let blocks_clone = blocks.clone();
    let bootstrap = bootstrap.to_vec();

    let task = tokio::spawn(async move {
        run_loop(
            swarm,
            blocks_clone,
            cmd_stream,
            notify_tx,
            shutdown_rx,
            addr_tx,
            bootstrap,
        )
        .await;
    });

    // Also need oneshot, so we poll the receiver after the spawn
    let listen_addr = addr_rx.await.context("swarm did not report listen address")?;

    Ok(TestNode {
        peer_id: pid,
        listen_addr,
        cmd_tx,
        notify_rx: Some(notify_rx),
        shutdown_tx: Some(shutdown_tx),
        _task: task,
    })
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

async fn run_loop(
    mut swarm: libp2p::Swarm<GuixP2PBehaviour>,
    blocks: BlockMap,
    mut cmd_rx: UnboundedReceiverStream<SwarmCommand>,
    notify_tx: UnboundedSender<SwarmNotification>,
    mut shutdown: oneshot::Receiver<()>,
    addr_tx: oneshot::Sender<Multiaddr>,
    bootstrap: Vec<Multiaddr>,
) {
    let mut addr_opt = Some(addr_tx);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
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
                SwarmEvent::ConnectionEstablished { .. } => {
                    // Bootstrap Kademlia routing table to enable provider lookups
                    if let Err(e) = swarm.behaviour_mut().kad.bootstrap() {
                        tracing::warn!("kad bootstrap error: {}", e);
                    }
                }
                // Provider discovery result → forward
                SwarmEvent::Behaviour(GuixP2PEvent::Kad(
                    kad::Event::OutboundQueryProgressed {
                        result: QueryResult::GetProviders(Ok(
                            GetProvidersOk::FoundProviders { key, providers, .. },
                        )),
                        ..
                    },
                )) => {
                    let _ = notify_tx.send(SwarmNotification::ProvidersFound {
                        hash: hex::encode(key.as_ref()),
                        peers: providers.iter().copied().collect(),
                    });
                }
                // Incoming block request → serve
                SwarmEvent::Behaviour(GuixP2PEvent::BlockExchange(
                    request_response::Event::Message {
                        peer: _,
                        message: request_response::Message::Request { request, channel, .. },
                        ..
                    },
                )) => {
                    let resp = serve(&blocks, &request);
                    let _ = swarm.behaviour_mut().block_exchange.send_response(channel, resp);
                }
                // Response to our block request → forward
                SwarmEvent::Behaviour(GuixP2PEvent::BlockExchange(
                    request_response::Event::Message {
                        peer,
                        message: request_response::Message::Response { response, .. },
                        ..
                    },
                )) => {
                    let _ = notify_tx.send(SwarmNotification::BlockResponse { peer, response });
                }
                _ => {}
            },
            Some(cmd) = cmd_rx.next() => match cmd {
                SwarmCommand::GetProviders { hash } => {
                    if let Ok(bytes) = hex::decode(hash) {
                        swarm.behaviour_mut().kad.get_providers(RecordKey::new(&bytes));
                    }
                }
                SwarmCommand::SendBlockRequest { peer, request } => {
                    let _ = swarm.behaviour_mut().block_exchange.send_request(&peer, request);
                }
            },
        }
    }
}

fn serve(blocks: &BlockMap, req: &BlockRequest) -> BlockResponse {
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
        }
        BlockRequest::GetBlocks { indices } => {
            for data in blocks.lock().unwrap().values() {
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
                return BlockResponse::Blocks { data: blks };
            }
            BlockResponse::Error { message: "no blocks seeded".into() }
        }
    }
}
