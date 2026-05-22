use std::time::{Duration, Instant};

use futures::StreamExt;
use libp2p::{Multiaddr, PeerId, multiaddr::Protocol, swarm::SwarmEvent};
use serde::Serialize;

use crate::runtime;

/// Result of an active outbound connectivity test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectivityTestReport {
    pub peer: String,
    pub address: String,
    pub success: bool,
    pub elapsed_ms: u128,
    pub detail: String,
}

/// Try to dial one peer multiaddr and report whether libp2p connected.
pub async fn test_peer_connectivity(
    keypair: &libp2p::identity::Keypair,
    peer_addr: &str,
    timeout: Duration,
) -> anyhow::Result<ConnectivityTestReport> {
    let addr: Multiaddr =
        peer_addr.parse().map_err(|e| anyhow::anyhow!("invalid multiaddr {peer_addr}: {e}"))?;
    let peer = peer_id_from_multiaddr(&addr);
    let started = Instant::now();
    let mut swarm = runtime::build_swarm_without_mdns(keypair)?;

    swarm.dial(addr.clone()).map_err(|e| anyhow::anyhow!("failed to start dial {addr}: {e}"))?;

    loop {
        let event = tokio::time::timeout(timeout.saturating_sub(started.elapsed()), swarm.next())
            .await
            .map_err(|_| anyhow::anyhow!("timed out dialing {addr}"))?;

        let Some(event) = event else {
            anyhow::bail!("swarm ended before dial completed");
        };

        match event {
            SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                let connected_addr = match endpoint {
                    libp2p::core::ConnectedPoint::Dialer { address, .. } => address.to_string(),
                    libp2p::core::ConnectedPoint::Listener { send_back_addr, .. } => {
                        send_back_addr.to_string()
                    },
                };
                return Ok(ConnectivityTestReport {
                    peer: peer_id.to_string(),
                    address: connected_addr.clone(),
                    success: true,
                    elapsed_ms: started.elapsed().as_millis(),
                    detail: format!("connected to {peer_id} at {connected_addr}"),
                });
            },
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                return Ok(ConnectivityTestReport {
                    peer: peer_id
                        .or(peer)
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "unknown".into()),
                    address: addr.to_string(),
                    success: false,
                    elapsed_ms: started.elapsed().as_millis(),
                    detail: format!("dial failed: {error}"),
                });
            },
            _ => {},
        }
    }
}

/// Format a connectivity test report for terminal output.
pub fn format_connectivity_test(report: &ConnectivityTestReport) -> String {
    let status = if report.success { "ok" } else { "error" };
    format!(
        "{status:5} connectivity-test {}\n      peer: {}\n      elapsed: {}ms\n",
        report.detail, report.peer, report.elapsed_ms
    )
}

fn peer_id_from_multiaddr(addr: &Multiaddr) -> Option<PeerId> {
    match addr.iter().last() {
        Some(Protocol::P2p(peer_id)) => Some(peer_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_id_from_multiaddr_reads_p2p_suffix() {
        let peer = PeerId::random();
        let addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/6881/p2p/{peer}").parse().unwrap();

        assert_eq!(peer_id_from_multiaddr(&addr), Some(peer));
    }

    #[test]
    fn format_connectivity_test_marks_failures() {
        let report = ConnectivityTestReport {
            peer: "unknown".to_string(),
            address: "/ip4/127.0.0.1/tcp/1".to_string(),
            success: false,
            elapsed_ms: 10,
            detail: "dial failed".to_string(),
        };

        assert!(format_connectivity_test(&report).starts_with("error"));
    }
}
