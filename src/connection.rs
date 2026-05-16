use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use libp2p::PeerId;

/// Tunable limits for peer dialing, retry backoff, and connection pruning.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub connect_timeout: Duration,
    pub max_retries: u32,
    pub backoff_base: Duration,
    pub health_check_interval: Duration,
    pub max_peers: usize,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        ConnectionConfig {
            connect_timeout: Duration::from_secs(10),
            max_retries: 3,
            backoff_base: Duration::from_secs(1),
            health_check_interval: Duration::from_secs(60),
            max_peers: 50,
        }
    }
}

#[derive(Debug)]
struct PeerState {
    retry_count: u32,
    last_attempt: Instant,
    pub last_active: Instant,
    connected: bool,
    addresses: HashSet<String>,
}

/// Snapshot of connection-manager state for one peer.
///
/// This is used by the dashboard and provider selection code so callers do not
/// need to inspect mutable connection-manager internals.
#[derive(Debug, Clone)]
pub struct PeerConnectionSnapshot {
    pub peer: PeerId,
    pub connected: bool,
    pub addresses: Vec<String>,
    pub last_active_secs_ago: u64,
}

/// Tracks peer connection state and retry backoff.
///
/// This manager is deliberately separate from `ReputationTracker`: connection
/// state answers "can we dial this peer now?", while reputation answers "how
/// reliable has this peer been when serving blocks?".
#[derive(Debug)]
pub struct ConnectionManager {
    config: ConnectionConfig,
    peers: HashMap<PeerId, PeerState>,
}

impl ConnectionManager {
    /// Create a connection manager with the given limits.
    pub fn new(config: ConnectionConfig) -> Self {
        ConnectionManager { config, peers: HashMap::new() }
    }

    /// Mark a peer as connected without adding new address metadata.
    pub fn on_connected(&mut self, peer: PeerId) {
        self.on_connected_with_addresses(peer, Vec::new());
    }

    /// Mark a peer as connected and merge any dialable addresses learned.
    pub fn on_connected_with_addresses(&mut self, peer: PeerId, addresses: Vec<String>) {
        let now = Instant::now();
        let entry = self.peers.entry(peer).or_insert(PeerState {
            retry_count: 0,
            last_attempt: now,
            last_active: now,
            connected: false,
            addresses: HashSet::new(),
        });
        entry.retry_count = 0;
        entry.last_active = now;
        entry.connected = true;
        entry.addresses.extend(addresses);
    }

    /// Mark a peer as disconnected while keeping address and retry history.
    pub fn on_disconnected(&mut self, peer: PeerId) {
        if let Some(state) = self.peers.get_mut(&peer) {
            state.last_active = Instant::now();
            state.connected = false;
        }
    }

    /// Mark recent successful peer activity.
    pub fn on_active(&mut self, peer: PeerId) {
        let state = self.peer_state(peer);
        state.last_active = Instant::now();
    }

    /// Store a dialable address learned from identify, mDNS, or dialing.
    pub fn add_address(&mut self, peer: PeerId, address: String) {
        let state = self.peer_state(peer);
        state.addresses.insert(address);
        state.last_active = Instant::now();
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    pub fn connected_count(&self) -> usize {
        self.peers.values().filter(|state| state.connected).count()
    }

    /// Return a read-only dashboard/API snapshot sorted by each caller as needed.
    pub fn peer_snapshots(&self) -> Vec<PeerConnectionSnapshot> {
        let now = Instant::now();
        self.peers
            .iter()
            .map(|(peer, state)| {
                let mut addresses: Vec<String> = state.addresses.iter().cloned().collect();
                addresses.sort();
                PeerConnectionSnapshot {
                    peer: *peer,
                    connected: state.connected,
                    addresses,
                    last_active_secs_ago: now.duration_since(state.last_active).as_secs(),
                }
            })
            .collect()
    }

    /// Return true if the peer is below retry limits or its backoff expired.
    pub fn can_connect(&self, peer: &PeerId) -> bool {
        if self.peers.len() >= self.config.max_peers {
            return false;
        }

        match self.peers.get(peer) {
            Some(state) => {
                state.retry_count < self.config.max_retries || self.backoff_expired(state)
            },
            None => true,
        }
    }

    /// Return the remaining backoff before another dial should be attempted.
    pub fn next_retry_delay(&self, peer: &PeerId) -> Option<Duration> {
        self.peers.get(peer).and_then(|state| {
            if state.retry_count >= self.config.max_retries {
                let elapsed = state.last_attempt.elapsed();
                let max_backoff = self.config.backoff_base * 2u32.pow(self.config.max_retries);
                if elapsed < max_backoff {
                    return Some(max_backoff - elapsed);
                }
            }
            None
        })
    }

    /// Record an outbound dial or handshake attempt.
    pub fn record_attempt(&mut self, peer: PeerId) {
        let now = Instant::now();
        let entry = self.peers.entry(peer).or_insert(PeerState {
            retry_count: 0,
            last_attempt: now,
            last_active: now,
            connected: false,
            addresses: HashSet::new(),
        });
        entry.retry_count += 1;
        entry.last_attempt = now;
    }

    /// Remove peers that have been inactive beyond the health-check window.
    pub fn prune_dead(&mut self) -> Vec<PeerId> {
        let now = Instant::now();
        let threshold = now - self.config.health_check_interval * 2;

        let mut dead = Vec::new();
        self.peers.retain(|peer, state| {
            if state.last_active < threshold {
                dead.push(*peer);
                false
            } else {
                true
            }
        });
        dead
    }

    fn backoff_expired(&self, state: &PeerState) -> bool {
        let delay = self.config.backoff_base * 2u32.pow(state.retry_count.min(6));
        state.last_attempt.elapsed() >= delay
    }

    fn peer_state(&mut self, peer: PeerId) -> &mut PeerState {
        let now = Instant::now();
        self.peers.entry(peer).or_insert(PeerState {
            retry_count: 0,
            last_attempt: now,
            last_active: now,
            connected: false,
            addresses: HashSet::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_peer_can_connect() {
        let mgr = ConnectionManager::new(ConnectionConfig::default());
        assert!(mgr.can_connect(&PeerId::random()));
    }

    #[test]
    fn test_retry_limit_reached() {
        let config = ConnectionConfig { max_retries: 2, ..Default::default() };
        let mut mgr = ConnectionManager::new(config);
        let peer = PeerId::random();

        mgr.record_attempt(peer);
        assert!(mgr.can_connect(&peer));
        mgr.record_attempt(peer);
        assert!(!mgr.can_connect(&peer));
    }

    #[test]
    fn test_success_resets_retries() {
        let mut mgr = ConnectionManager::new(ConnectionConfig::default());
        let peer = PeerId::random();
        mgr.record_attempt(peer);
        mgr.record_attempt(peer);
        mgr.on_connected(peer);
        assert!(mgr.can_connect(&peer));
    }

    #[test]
    fn test_prune_dead_removes_stale_peers() {
        let mut mgr = ConnectionManager::new(ConnectionConfig::default());
        let peer = PeerId::random();

        mgr.record_attempt(peer);
        if let Some(state) = mgr.peers.get_mut(&peer) {
            state.last_active = Instant::now() - Duration::from_secs(3600);
        }

        let dead = mgr.prune_dead();
        assert!(dead.contains(&peer));
        assert!(mgr.peers.is_empty());
    }

    #[test]
    fn connected_count_excludes_disconnected_peers() {
        let mut mgr = ConnectionManager::new(ConnectionConfig::default());
        let connected = PeerId::random();
        let disconnected = PeerId::random();

        mgr.on_connected(connected);
        mgr.on_connected(disconnected);
        mgr.on_disconnected(disconnected);

        assert_eq!(mgr.peer_count(), 2);
        assert_eq!(mgr.connected_count(), 1);
        let snapshots = mgr.peer_snapshots();
        assert!(snapshots.iter().any(|snapshot| snapshot.peer == connected && snapshot.connected));
        assert!(
            snapshots.iter().any(|snapshot| snapshot.peer == disconnected && !snapshot.connected)
        );
    }

    #[test]
    fn peer_snapshots_deduplicate_addresses() {
        let mut mgr = ConnectionManager::new(ConnectionConfig::default());
        let peer = PeerId::random();
        let addr = "/ip4/127.0.0.1/tcp/6881".to_string();

        mgr.on_connected_with_addresses(peer, vec![addr.clone(), addr.clone()]);
        mgr.add_address(peer, addr.clone());

        let snapshots = mgr.peer_snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].addresses, vec![addr]);
        assert_eq!(snapshots[0].last_active_secs_ago, 0);
    }

    #[test]
    fn test_max_peers_enforced() {
        let config = ConnectionConfig { max_peers: 2, ..Default::default() };
        let mut mgr = ConnectionManager::new(config);

        let p1 = PeerId::random();
        let p2 = PeerId::random();
        let p3 = PeerId::random();

        mgr.record_attempt(p1);
        mgr.record_attempt(p2);
        // p3 should be blocked when we're at max
        assert!(!mgr.can_connect(&p3));
    }
}
