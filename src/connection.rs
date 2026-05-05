use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use libp2p::PeerId;

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
}

#[derive(Debug)]
pub struct ConnectionManager {
    config: ConnectionConfig,
    peers: HashMap<PeerId, PeerState>,
}

impl ConnectionManager {
    pub fn new(config: ConnectionConfig) -> Self {
        ConnectionManager { config, peers: HashMap::new() }
    }

    pub fn on_connected(&mut self, peer: PeerId) {
        let now = Instant::now();
        let entry = self.peers.entry(peer).or_insert(PeerState {
            retry_count: 0,
            last_attempt: now,
            last_active: now,
        });
        entry.retry_count = 0;
        entry.last_active = now;
    }

    pub fn on_disconnected(&mut self, peer: PeerId) {
        if let Some(state) = self.peers.get_mut(&peer) {
            state.last_active = Instant::now();
        }
    }

    pub fn on_active(&mut self, peer: PeerId) {
        if let Some(state) = self.peers.get_mut(&peer) {
            state.last_active = Instant::now();
        }
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

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

    pub fn record_attempt(&mut self, peer: PeerId) {
        let now = Instant::now();
        let entry = self.peers.entry(peer).or_insert(PeerState {
            retry_count: 0,
            last_attempt: now,
            last_active: now,
        });
        entry.retry_count += 1;
        entry.last_attempt = now;
    }

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
