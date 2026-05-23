use std::{collections::HashMap, sync::Arc, time::Duration};

use futures::StreamExt;
use libp2p::{
    Multiaddr, SwarmBuilder,
    kad::QueryId,
    noise, quic, request_response,
    swarm::{DialError, SwarmEvent},
    tcp, yamux,
};

use crate::{
    bandwidth::BandwidthLimiter,
    behaviour::{self, GuixP2PBehaviour, GuixP2PEvent},
    channel::{NotifyTx, SwarmCommand, SwarmNotification},
    connection::ConnectionManager,
    dashboard, dht,
    nar_store::NarStore,
    peer_store::{self, PeerStore},
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
    bandwidth_limiter: Arc<BandwidthLimiter>,
    peer_store: Option<Arc<std::sync::Mutex<PeerStore>>>,
) {
    let mut prune_tick = tokio::time::interval(Duration::from_secs(300));
    let (response_tx, mut response_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut active_provider_queries: HashMap<QueryId, String> = HashMap::new();

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
                        dht::handle_kad_event(
                            &cache,
                            &notify_tx,
                            &event_tx,
                            &mut active_provider_queries,
                            e,
                        );
                    },
                    SwarmEvent::Behaviour(GuixP2PEvent::BlockExchange(e)) => {
                        let ctx = BlockExchangeContext {
                            notify_tx: &notify_tx,
                            reputation: &reputation,
                            conn_mgr: &conn_mgr,
                            nar_store: &nar_store,
                            bandwidth_limiter: &bandwidth_limiter,
                            response_tx: &response_tx,
                        };
                        handle_block_exchange(ctx, e);
                    },
                    SwarmEvent::Behaviour(GuixP2PEvent::Mdns(libp2p::mdns::Event::Discovered(list))) => {
                        for (peer_id, addr) in list {
                            tracing::info!("Discovered LAN peer {} at {}", peer_id, addr);
                            if peer_store::is_peer_address(&addr) {
                                record_peer_address(&peer_store, peer_id, &addr);
                                swarm.behaviour_mut().kad.add_address(&peer_id, addr);
                            } else {
                                tracing::debug!(
                                    "ignored non-dialable discovered peer address {} for {}",
                                    addr,
                                    peer_id
                                );
                            }
                        }
                    },
                    SwarmEvent::Behaviour(GuixP2PEvent::Identify(e)) => {
                        if let libp2p::identify::Event::Received { peer_id, info, .. } = *e {
                            for addr in info.listen_addrs {
                                tracing::debug!("Identify learned peer {} at {}", peer_id, addr);
                                if peer_store::is_public_peer_address(&addr) {
                                    conn_mgr.lock().unwrap().add_address(peer_id, addr.to_string());
                                    record_peer_address(&peer_store, peer_id, &addr);
                                    swarm.behaviour_mut().kad.add_address(&peer_id, addr);
                                } else {
                                    tracing::debug!(
                                        "ignored non-public identify address {} for {}",
                                        addr,
                                        peer_id
                                    );
                                }
                            }
                        }
                    },
                    SwarmEvent::ConnectionEstablished {
                        peer_id,
                        endpoint: libp2p::core::ConnectedPoint::Dialer { address, .. },
                        ..
                    } => {
                        conn_mgr
                            .lock()
                            .unwrap()
                            .on_connected_with_addresses(peer_id, vec![address.to_string()]);
                        record_peer_address(&peer_store, peer_id, &address);
                        let _ = event_tx.send(dashboard::DashboardEvent::PeerConnected {
                            peer_id: peer_id.to_string(),
                            addresses: vec![address.to_string()],
                        });
                        announce_seeded_nars(&mut swarm, &nar_store, &event_tx, "peer-connected");
                        tracing::debug!("Connection established with {}", peer_id);
                    },
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        conn_mgr.lock().unwrap().on_connected(peer_id);
                        let _ = event_tx.send(dashboard::DashboardEvent::PeerConnected {
                            peer_id: peer_id.to_string(),
                            addresses: vec![],
                        });
                        announce_seeded_nars(&mut swarm, &nar_store, &event_tx, "peer-connected");
                        tracing::debug!("Connection established with {}", peer_id);
                    },
                    SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                        conn_mgr.lock().unwrap().on_disconnected(peer_id);
                        let reason = cause.as_ref().map(|cause| cause.to_string());
                        let _ = event_tx.send(dashboard::DashboardEvent::PeerDisconnected {
                            peer_id: peer_id.to_string(),
                            reason,
                        });
                        tracing::debug!("Connection closed with {}", peer_id);
                    },
                    SwarmEvent::Dialing { peer_id, .. } => {
                        let _ = event_tx.send(dashboard::DashboardEvent::PeerDialStarted {
                            peer_id: peer_id.map(|peer_id| peer_id.to_string()),
                        });
                    },
                    SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                        let reason = error.to_string();
                        if let Some(peer_id) = peer_id {
                            remove_failed_dial_addresses(
                                &peer_store,
                                &conn_mgr,
                                &mut swarm,
                                peer_id,
                                &error,
                            );
                        }
                        let _ = event_tx.send(dashboard::DashboardEvent::PeerDialFailed {
                            peer_id: peer_id.map(|peer_id| peer_id.to_string()),
                            reason,
                        });
                        tracing::debug!("Outgoing connection failed: {}", error);
                    },
                    SwarmEvent::IncomingConnection { send_back_addr, .. } => {
                        let _ = event_tx.send(dashboard::DashboardEvent::PeerInboundStarted {
                            address: send_back_addr.to_string(),
                        });
                    },
                    SwarmEvent::IncomingConnectionError {
                        peer_id,
                        send_back_addr,
                        error,
                        ..
                    } => {
                        let _ = event_tx.send(dashboard::DashboardEvent::PeerInboundFailed {
                            peer_id: peer_id.map(|peer_id| peer_id.to_string()),
                            address: send_back_addr.to_string(),
                            reason: error.to_string(),
                        });
                    },
                    other => handle_swarm_event(other),
                }
            }
            Some(cmd) = cmd_rx.next() => {
                handle_swarm_command(&mut swarm, cmd, &event_tx, &mut active_provider_queries);
            }
            Some(pending) = response_rx.recv() => {
                let _ = swarm
                    .behaviour_mut()
                    .block_exchange
                    .send_response(pending.channel, pending.response);
                if let Some(event) = pending.event {
                    let _ = event_tx.send(event);
                }
            }
            Some(hash) = query_rx.recv() => {
                let key = libp2p::kad::RecordKey::new(&dht::extract_hash_bytes(&hash));
                let _ = event_tx.send(dashboard::DashboardEvent::ProviderLookupStarted {
                    nar_hash: hash.clone(),
                });
                let query_id = swarm.behaviour_mut().kad.get_providers(key);
                active_provider_queries.insert(query_id, hash.clone());
                tracing::debug!("DHT get_providers for hash={}", hash);
            }
        }
    }
}

fn announce_seeded_nars(
    swarm: &mut libp2p::Swarm<GuixP2PBehaviour>,
    nar_store: &Arc<std::sync::Mutex<NarStore>>,
    event_tx: &dashboard::EventBus,
    reason: &str,
) {
    let hashes = nar_store.lock().unwrap().seeded_hashes();
    for hash in hashes {
        announce_nar(swarm, &hash, event_tx, reason);
    }
}

fn announce_nar(
    swarm: &mut libp2p::Swarm<GuixP2PBehaviour>,
    hash: &str,
    event_tx: &dashboard::EventBus,
    reason: &str,
) {
    if let Ok(bytes) = hex::decode(hash) {
        let _ = event_tx.send(dashboard::DashboardEvent::ProviderAnnounceStarted {
            nar_hash: hash.to_string(),
            reason: reason.to_string(),
        });
        let key = libp2p::kad::RecordKey::new(&bytes);
        if let Err(e) = swarm.behaviour_mut().kad.start_providing(key) {
            let _ = event_tx.send(dashboard::DashboardEvent::ProviderAnnounceFinished {
                nar_hash: hash.to_string(),
                result: "failed".to_string(),
                reason: Some(e.to_string()),
            });
            tracing::warn!(
                reason = %reason,
                "failed to announce nar {}: {}",
                &hash[..16.min(hash.len())],
                e
            );
        } else {
            tracing::info!(
                reason = %reason,
                "announced nar {}.. in DHT",
                &hash[..16.min(hash.len())]
            );
        }
    }
}

fn record_peer_address(
    peer_store: &Option<Arc<std::sync::Mutex<PeerStore>>>,
    peer_id: libp2p::PeerId,
    address: &libp2p::Multiaddr,
) {
    if let Some(store) = peer_store {
        store.lock().unwrap().record_address(peer_id, address);
    }
}

fn remove_failed_dial_addresses(
    peer_store: &Option<Arc<std::sync::Mutex<PeerStore>>>,
    conn_mgr: &Arc<std::sync::Mutex<ConnectionManager>>,
    swarm: &mut libp2p::Swarm<GuixP2PBehaviour>,
    peer_id: libp2p::PeerId,
    error: &DialError,
) {
    for address in failed_dial_addresses(error) {
        if let Some(store) = peer_store {
            store.lock().unwrap().remove_address(peer_id, &address);
        }
        conn_mgr.lock().unwrap().remove_address(peer_id, &address.to_string());
        swarm.behaviour_mut().kad.remove_address(&peer_id, &address);
    }
}

fn failed_dial_addresses(error: &DialError) -> Vec<Multiaddr> {
    match error {
        DialError::LocalPeerId { address } | DialError::WrongPeerId { address, .. } => {
            vec![address.clone()]
        },
        DialError::Transport(errors) => errors.iter().map(|(address, _)| address.clone()).collect(),
        DialError::NoAddresses
        | DialError::DialPeerConditionFalse(_)
        | DialError::Aborted
        | DialError::Denied { .. } => Vec::new(),
    }
}

fn handle_swarm_command(
    swarm: &mut libp2p::Swarm<GuixP2PBehaviour>,
    cmd: SwarmCommand,
    event_tx: &dashboard::EventBus,
    active_provider_queries: &mut HashMap<QueryId, String>,
) {
    match cmd {
        SwarmCommand::GetProviders { hash } => {
            let key_bytes = dht::extract_hash_bytes(&hash);
            let key = libp2p::kad::RecordKey::new(&key_bytes);
            let _ = event_tx
                .send(dashboard::DashboardEvent::ProviderLookupStarted { nar_hash: hash.clone() });
            let query_id = swarm.behaviour_mut().kad.get_providers(key);
            active_provider_queries.insert(query_id, hash.clone());
            tracing::debug!("DHT get_providers for hash={}", hash);
        },
        SwarmCommand::SendBlockRequest { peer, request } => {
            let req_id = swarm.behaviour_mut().block_exchange.send_request(&peer, request);
            tracing::debug!("Sent block request {:?} to peer={}", req_id, peer);
        },
        SwarmCommand::StartProviding { hash } => {
            announce_nar(swarm, &hash, event_tx, "post-download");
        },
    }
}

struct BlockExchangeContext<'a> {
    notify_tx: &'a NotifyTx,
    reputation: &'a Arc<std::sync::Mutex<ReputationTracker>>,
    conn_mgr: &'a Arc<std::sync::Mutex<ConnectionManager>>,
    nar_store: &'a Arc<std::sync::Mutex<NarStore>>,
    bandwidth_limiter: &'a Arc<BandwidthLimiter>,
    response_tx: &'a tokio::sync::mpsc::UnboundedSender<PendingBlockResponse>,
}

struct PendingBlockResponse {
    channel: request_response::ResponseChannel<BlockResponse>,
    response: BlockResponse,
    event: Option<dashboard::DashboardEvent>,
}

fn handle_block_exchange(
    ctx: BlockExchangeContext<'_>,
    event: request_response::Event<BlockRequest, BlockResponse>,
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
                let resp = ctx.nar_store.lock().unwrap().handle_request(&request);
                if let Some(resp) = resp {
                    let bytes = response_size_bytes(&resp);
                    let limiter = ctx.bandwidth_limiter.clone();
                    let response_tx = ctx.response_tx.clone();
                    let event = dashboard::DashboardEvent::BlockServed {
                        nar_hash: nar_hash_for_event,
                        peer_id: peer.to_string(),
                        indices: indices_for_event,
                    };
                    tokio::spawn(async move {
                        limiter.wait_for_upload(bytes).await;
                        let _ = response_tx.send(PendingBlockResponse {
                            channel,
                            response: resp,
                            event: Some(event),
                        });
                    });
                    tracing::trace!("Queued block response to {}", peer);
                } else {
                    tracing::trace!("Could not serve block request (request_id={})", request_id);
                }
            },
            request_response::Message::Response { response, .. } => {
                let _ = ctx.notify_tx.send(SwarmNotification::BlockResponse { peer, response });
                ctx.conn_mgr.lock().unwrap().on_active(peer);
                ctx.reputation.lock().unwrap().record_success(peer, 0);
            },
        },
        request_response::Event::OutboundFailure { peer, error, .. } => {
            tracing::warn!("Outbound request failed for {}: {:?}", peer, error);
            let _ = ctx.notify_tx.send(SwarmNotification::BlockRequestFailed { peer });
            ctx.reputation.lock().unwrap().record_failure(peer);
        },
        other => {
            tracing::trace!("BlockExchange event: {:?}", other);
        },
    }
}

fn response_size_bytes(response: &BlockResponse) -> u64 {
    match response {
        BlockResponse::HandshakeReply { block_hashes, .. } => {
            block_hashes.iter().map(|hash| hash.len() as u64).sum()
        },
        BlockResponse::Blocks { data } => data.iter().map(|block| block.data.len() as u64).sum(),
        BlockResponse::Error { message } => message.len() as u64,
    }
}

pub fn build_swarm(
    keypair: &libp2p::identity::Keypair,
) -> anyhow::Result<libp2p::Swarm<GuixP2PBehaviour>> {
    build_swarm_with_mdns(keypair, true)
}

/// Build a libp2p swarm without LAN mDNS discovery.
pub fn build_swarm_without_mdns(
    keypair: &libp2p::identity::Keypair,
) -> anyhow::Result<libp2p::Swarm<GuixP2PBehaviour>> {
    build_swarm_with_mdns(keypair, false)
}

fn build_swarm_with_mdns(
    keypair: &libp2p::identity::Keypair,
    enable_mdns: bool,
) -> anyhow::Result<libp2p::Swarm<GuixP2PBehaviour>> {
    let mut quic_config = quic::Config::new(keypair);
    quic_config.max_idle_timeout = 30_000;

    let swarm = SwarmBuilder::with_existing_identity(keypair.clone())
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_quic_config(|_| quic_config)
        .with_dns()?
        .with_behaviour(|keypair| {
            if enable_mdns {
                Ok(behaviour::create_swarm_behaviour(keypair))
            } else {
                Ok(behaviour::create_swarm_behaviour_without_mdns(keypair))
            }
        })?
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
