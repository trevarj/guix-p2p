use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use libp2p::{SwarmBuilder, noise, quic, request_response, swarm::SwarmEvent, tcp, yamux};

use crate::{
    behaviour::{self, GuixP2PBehaviour, GuixP2PEvent},
    channel::{NotifyTx, SwarmCommand, SwarmNotification},
    connection::ConnectionManager,
    dashboard, dht,
    nar_store::NarStore,
    reputation::ReputationTracker,
    swarm::codec::{BlockRequest, BlockResponse},
};

#[allow(clippy::too_many_arguments)]
pub async fn run_swarm_task(
    mut swarm: libp2p::Swarm<GuixP2PBehaviour>,
    cache: dht::ProviderCache,
    notify_tx: NotifyTx,
    mut cmd_rx: tokio_stream::wrappers::UnboundedReceiverStream<SwarmCommand>,
    mut query_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    reputation: Arc<std::sync::Mutex<ReputationTracker>>,
    conn_mgr: Arc<std::sync::Mutex<ConnectionManager>>,
    event_tx: dashboard::EventBus,
    nar_store: Arc<std::sync::Mutex<NarStore>>,
) {
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
                        handle_block_exchange(
                            &notify_tx,
                            &event_tx,
                            e,
                            &reputation,
                            &conn_mgr,
                            &mut swarm,
                            &nar_store,
                        );
                    },
                    SwarmEvent::Behaviour(GuixP2PEvent::Mdns(libp2p::mdns::Event::Discovered(list))) => {
                        for (peer_id, addr) in list {
                            tracing::info!("Discovered LAN peer {} at {}", peer_id, addr);
                            swarm.behaviour_mut().kad.add_address(&peer_id, addr);
                        }
                    },
                    SwarmEvent::Behaviour(GuixP2PEvent::Identify(e)) => {
                        if let libp2p::identify::Event::Received { peer_id, info, .. } = *e {
                            for addr in info.listen_addrs {
                                tracing::debug!("Identify learned peer {} at {}", peer_id, addr);
                                swarm.behaviour_mut().kad.add_address(&peer_id, addr);
                            }
                        }
                    },
                    SwarmEvent::ConnectionEstablished {
                        peer_id,
                        endpoint: libp2p::core::ConnectedPoint::Dialer { address, .. },
                        ..
                    } => {
                        conn_mgr.lock().unwrap().on_connected(peer_id);
                        let _ = event_tx.send(dashboard::DashboardEvent::PeerConnected {
                            peer_id: peer_id.to_string(),
                            addresses: vec![address.to_string()],
                        });
                        tracing::debug!("Connection established with {}", peer_id);
                    },
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        conn_mgr.lock().unwrap().on_connected(peer_id);
                        let _ = event_tx.send(dashboard::DashboardEvent::PeerConnected {
                            peer_id: peer_id.to_string(),
                            addresses: vec![],
                        });
                        tracing::debug!("Connection established with {}", peer_id);
                    },
                    SwarmEvent::ConnectionClosed { peer_id, .. } => {
                        conn_mgr.lock().unwrap().on_disconnected(peer_id);
                        let _ = event_tx.send(dashboard::DashboardEvent::PeerDisconnected {
                            peer_id: peer_id.to_string(),
                        });
                        tracing::debug!("Connection closed with {}", peer_id);
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
        SwarmCommand::StartProviding { hash } => {
            if let Ok(bytes) = hex::decode(&hash) {
                let key = libp2p::kad::RecordKey::new(&bytes);
                if let Err(e) = swarm.behaviour_mut().kad.start_providing(key) {
                    tracing::warn!(
                        "failed to start providing nar {}: {}",
                        &hash[..16.min(hash.len())],
                        e
                    );
                } else {
                    tracing::info!(
                        "announced nar {}.. in DHT (post-download)",
                        &hash[..16.min(hash.len())]
                    );
                }
            }
        },
    }
}

fn handle_block_exchange(
    notify_tx: &NotifyTx,
    event_bus: &dashboard::EventBus,
    event: request_response::Event<BlockRequest, BlockResponse>,
    reputation: &Arc<std::sync::Mutex<ReputationTracker>>,
    conn_mgr: &Arc<std::sync::Mutex<ConnectionManager>>,
    swarm: &mut libp2p::Swarm<GuixP2PBehaviour>,
    nar_store: &Arc<std::sync::Mutex<NarStore>>,
) {
    match event {
        request_response::Event::Message { peer, message, .. } => match message {
            request_response::Message::Request { request_id, request, channel, .. } => {
                let nar_hash_for_event = match &request {
                    BlockRequest::Handshake { nar_hash } => hex::encode(nar_hash),
                    BlockRequest::GetBlocks { nar_hash, .. } => hex::encode(nar_hash),
                };
                let indices_for_event = match &request {
                    BlockRequest::GetBlocks { indices, .. } => indices.clone(),
                    BlockRequest::Handshake { .. } => vec![],
                };
                let resp = nar_store.lock().unwrap().handle_request(&request);
                if let Some(resp) = resp {
                    let _ = swarm.behaviour_mut().block_exchange.send_response(channel, resp);
                    tracing::trace!("Served block request to {}", peer);
                    let _ = event_bus.send(dashboard::DashboardEvent::BlockServed {
                        nar_hash: nar_hash_for_event,
                        peer_id: peer.to_string(),
                        indices: indices_for_event,
                    });
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

pub fn build_swarm(
    keypair: &libp2p::identity::Keypair,
) -> anyhow::Result<libp2p::Swarm<GuixP2PBehaviour>> {
    let mut quic_config = quic::Config::new(keypair);
    quic_config.max_idle_timeout = 30_000;

    let swarm = SwarmBuilder::with_existing_identity(keypair.clone())
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_quic_config(|_| quic_config)
        .with_behaviour(|keypair| Ok(behaviour::create_swarm_behaviour(keypair)))?
        .build();

    Ok(swarm)
}

fn handle_swarm_event(event: libp2p::swarm::SwarmEvent<GuixP2PEvent>) {
    match event {
        SwarmEvent::NewListenAddr { address, .. } => {
            tracing::info!("Swarm listening on {}", address);
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
        SwarmEvent::Behaviour(GuixP2PEvent::Kad(_)) => {
            tracing::trace!("Kademlia event");
        },
        SwarmEvent::Behaviour(GuixP2PEvent::BlockExchange(_)) => {
            tracing::trace!("Block exchange event");
        },
        other => {
            tracing::trace!("Swarm event: {:?}", other);
        },
    }
}
