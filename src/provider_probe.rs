use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use libp2p::{Multiaddr, PeerId, multiaddr::Protocol};
use serde::Serialize;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    channel::{NotifyRx, SwarmCommand, SwarmNotification},
    config::Config,
    connection::ConnectionManager,
    dht,
    swarm::codec::{BlockRequest, BlockResponse},
};

/// Active provider lookup and fallback-handshake probe.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderLookupProbeReport {
    pub nar_hash: String,
    pub dht_providers: Vec<String>,
    pub fallback_candidates: Vec<String>,
    pub handshake_peer: Option<String>,
    pub block_count: Option<u32>,
    pub success: bool,
    pub elapsed_ms: u128,
    pub detail: String,
}

/// Query Kad providers, then verify whether any discovered or fallback peer
/// actually has the NAR through the normal block handshake.
pub async fn test_provider_lookup(
    config: &Config,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    notify_rx: &mut NotifyRx,
    conn_mgr: &Arc<Mutex<ConnectionManager>>,
    nar_hash: &str,
    timeout: Duration,
) -> ProviderLookupProbeReport {
    let started = Instant::now();
    let hash_bytes = dht::extract_hash_bytes(nar_hash);
    let dht_key = hex::encode(hash_bytes);
    let _ = cmd_tx.send(SwarmCommand::GetProviders { hash: dht_key.clone() });

    let dht_providers = collect_providers(notify_rx, &dht_key, timeout / 2).await;
    let fallback_candidates = fallback_candidates(config, conn_mgr, &dht_providers);
    let handshake_candidates = merge_peers(&dht_providers, &fallback_candidates);

    for peer in &handshake_candidates {
        let request = BlockRequest::Handshake { nar_hash: hash_bytes.to_vec() };
        let _ = cmd_tx.send(SwarmCommand::SendBlockRequest { peer: *peer, request });
    }

    let handshake = wait_for_handshake(
        notify_rx,
        &handshake_candidates,
        timeout.saturating_sub(started.elapsed()),
    )
    .await;

    match handshake {
        Some((peer, block_count)) if block_count > 0 => ProviderLookupProbeReport {
            nar_hash: dht_key,
            dht_providers: stringify_peers(&dht_providers),
            fallback_candidates: stringify_peers(&fallback_candidates),
            handshake_peer: Some(peer.to_string()),
            block_count: Some(block_count),
            success: true,
            elapsed_ms: started.elapsed().as_millis(),
            detail: format!("provider handshake succeeded with {peer} ({block_count} blocks)"),
        },
        Some((peer, _)) => ProviderLookupProbeReport {
            nar_hash: dht_key,
            dht_providers: stringify_peers(&dht_providers),
            fallback_candidates: stringify_peers(&fallback_candidates),
            handshake_peer: Some(peer.to_string()),
            block_count: Some(0),
            success: false,
            elapsed_ms: started.elapsed().as_millis(),
            detail: format!("peer {peer} answered but did not have the NAR"),
        },
        None => ProviderLookupProbeReport {
            nar_hash: dht_key,
            dht_providers: stringify_peers(&dht_providers),
            fallback_candidates: stringify_peers(&fallback_candidates),
            handshake_peer: None,
            block_count: None,
            success: false,
            elapsed_ms: started.elapsed().as_millis(),
            detail: "no provider handshake succeeded".to_string(),
        },
    }
}

/// Format a provider lookup probe report for terminal output.
pub fn format_provider_lookup_probe(report: &ProviderLookupProbeReport) -> String {
    let status = if report.success { "ok" } else { "error" };
    format!(
        "{status:5} provider-lookup {}\n      nar: {}\n      dht providers: {}\n      fallback \
         candidates: {}\n      elapsed: {}ms\n",
        report.detail,
        report.nar_hash,
        report.dht_providers.len(),
        report.fallback_candidates.len(),
        report.elapsed_ms
    )
}

async fn collect_providers(
    notify_rx: &mut NotifyRx,
    dht_key: &str,
    timeout: Duration,
) -> Vec<PeerId> {
    let deadline = Instant::now() + timeout;
    let mut providers = Vec::new();
    let mut seen = HashSet::new();

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining.min(Duration::from_secs(1)), notify_rx.recv()).await {
            Ok(Ok(SwarmNotification::ProvidersFound { hash, peers })) if hash == dht_key => {
                providers.extend(peers.into_iter().filter(|peer| seen.insert(*peer)));
            },
            Ok(Ok(_)) => {},
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) | Err(_) => break,
        }
    }

    providers
}

async fn wait_for_handshake(
    notify_rx: &mut NotifyRx,
    candidates: &[PeerId],
    timeout: Duration,
) -> Option<(PeerId, u32)> {
    let candidate_set: HashSet<_> = candidates.iter().copied().collect();
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining.min(Duration::from_secs(1)), notify_rx.recv()).await {
            Ok(Ok(SwarmNotification::BlockResponse {
                peer,
                response: BlockResponse::HandshakeReply { block_count, .. },
            })) if candidate_set.contains(&peer) => return Some((peer, block_count)),
            Ok(Ok(_)) => {},
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) | Err(_) => break,
        }
    }

    None
}

fn fallback_candidates(
    config: &Config,
    conn_mgr: &Arc<Mutex<ConnectionManager>>,
    exclude: &[PeerId],
) -> Vec<PeerId> {
    let connected = conn_mgr.lock().unwrap().connected_peers();
    let bootstrap = bootstrap_peer_ids(&config.bootstrap_peers);
    let excluded: HashSet<_> = exclude.iter().copied().collect();
    merge_peers(&connected, &bootstrap)
        .into_iter()
        .filter(|peer| !excluded.contains(peer))
        .collect()
}

fn bootstrap_peer_ids(addrs: &[String]) -> Vec<PeerId> {
    let mut seen = HashSet::new();
    addrs
        .iter()
        .filter_map(|addr| bootstrap_peer_id(addr))
        .filter(|peer| seen.insert(*peer))
        .collect()
}

fn bootstrap_peer_id(addr: &str) -> Option<PeerId> {
    let addr = addr.parse::<Multiaddr>().ok()?;
    match addr.iter().last() {
        Some(Protocol::P2p(peer)) => Some(peer),
        _ => None,
    }
}

fn merge_peers(left: &[PeerId], right: &[PeerId]) -> Vec<PeerId> {
    let mut seen = HashSet::new();
    left.iter().chain(right.iter()).copied().filter(|peer| seen.insert(*peer)).collect()
}

fn stringify_peers(peers: &[PeerId]) -> Vec<String> {
    peers.iter().map(ToString::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_probe_report_formats_counts() {
        let report = ProviderLookupProbeReport {
            nar_hash: "abc".to_string(),
            dht_providers: vec!["peer-a".to_string()],
            fallback_candidates: vec!["peer-b".to_string()],
            handshake_peer: Some("peer-b".to_string()),
            block_count: Some(1),
            success: true,
            elapsed_ms: 42,
            detail: "ok".to_string(),
        };

        let formatted = format_provider_lookup_probe(&report);

        assert!(formatted.starts_with("ok"));
        assert!(formatted.contains("dht providers: 1"));
        assert!(formatted.contains("fallback candidates: 1"));
    }

    #[test]
    fn bootstrap_peer_ids_extracts_only_p2p_multiaddrs() {
        let peer = PeerId::random();
        let addrs = vec![
            format!("/dns4/bootstrap.example/tcp/443/p2p/{peer}"),
            "/dns4/bootstrap.example/tcp/443".to_string(),
            format!("/dns4/bootstrap.example/tcp/443/p2p/{peer}"),
        ];

        assert_eq!(bootstrap_peer_ids(&addrs), vec![peer]);
    }
}
