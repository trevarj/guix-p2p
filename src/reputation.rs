use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};

use libp2p::PeerId;

/// Runtime scorecard for a peer that has participated in block exchange.
///
/// Reputation is intentionally transport-only. It ranks peers for reliability
/// and backoff decisions, but it must not be used as build or metadata trust.
#[derive(Debug, Clone)]
pub struct PeerScore {
    pub completed: u32,
    pub failed: u32,
    pub bytes_served: u64,
    pub last_seen: Instant,
}

impl Default for PeerScore {
    fn default() -> Self {
        PeerScore { completed: 0, failed: 0, bytes_served: 0, last_seen: Instant::now() }
    }
}

impl PeerScore {
    /// Compute a bounded reliability score for provider ordering.
    ///
    /// New peers are neutral through `ReputationTracker::score`; once a peer
    /// has history, successful transfers, recent activity, and bytes served
    /// raise the score while failures lower it.
    pub fn score(&self, now: Instant) -> f64 {
        let base = self.completed as f64 / (self.completed + self.failed + 1) as f64;

        let age_secs = now.duration_since(self.last_seen).as_secs_f64();
        let decay = (-age_secs / 3600.0).exp();

        let byte_bonus =
            if self.bytes_served > 0 { (self.bytes_served as f64).ln() / 20.0 } else { 0.0 };

        (base * decay + byte_bonus).clamp(0.0, 1.0)
    }
}

/// Persistent peer reliability tracker used by the downloader.
///
/// Scores are saved under the guix-p2p cache directory and restored across
/// daemon restarts. The tracker only describes how well a peer served blocks;
/// it does not authenticate narinfo, attestations, or build outputs.
#[derive(Debug)]
pub struct ReputationTracker {
    peers: HashMap<PeerId, PeerScore>,
    ban_threshold: u32,
}

impl ReputationTracker {
    /// Create an empty tracker that bans peers after `ban_threshold` failures.
    pub fn new(ban_threshold: u32) -> Self {
        ReputationTracker { peers: HashMap::new(), ban_threshold }
    }

    /// Record a successful interaction with a peer.
    pub fn record_success(&mut self, peer: PeerId, bytes: u64) {
        let entry = self.peers.entry(peer).or_default();
        entry.completed += 1;
        entry.bytes_served += bytes;
        entry.last_seen = Instant::now();
    }

    /// Record a failed interaction with a peer.
    pub fn record_failure(&mut self, peer: PeerId) {
        let entry = self.peers.entry(peer).or_default();
        entry.failed += 1;
        entry.last_seen = Instant::now();
    }

    /// Return true when a peer has crossed the local failure threshold.
    pub fn is_banned(&self, peer: &PeerId) -> bool {
        self.peers.get(peer).map(|s| s.failed >= self.ban_threshold).unwrap_or(false)
    }

    /// Return the peer's current reliability score, or neutral for unknown peers.
    pub fn score(&self, peer: &PeerId) -> f64 {
        let now = Instant::now();
        self.peers.get(peer).map(|s| s.score(now)).unwrap_or(0.5)
    }

    pub fn peers(&self) -> Vec<(PeerId, f64)> {
        let now = Instant::now();
        self.peers.iter().map(|(peer, score)| (*peer, score.score(now))).collect()
    }

    pub fn peer_entries(&self) -> Vec<(PeerId, PeerScore)> {
        self.peers.iter().map(|(k, v)| (*k, v.clone())).collect()
    }

    /// Sort peer IDs in-place from most to least reliable.
    pub fn sort_by_score(&self, peers: &mut [PeerId]) {
        let now = Instant::now();
        peers.sort_by(|a, b| {
            let sa = self.peers.get(a).map(|s| s.score(now)).unwrap_or(0.5);
            let sb = self.peers.get(b).map(|s| s.score(now)).unwrap_or(0.5);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Return the best non-banned peers, preserving only the top `max` by score.
    pub fn best_peers(&self, peers: &[PeerId], max: usize) -> Vec<PeerId> {
        let mut scored: Vec<(PeerId, f64)> =
            peers.iter().filter(|p| !self.is_banned(p)).map(|p| (*p, self.score(p))).collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(max).map(|(p, _)| p).collect()
    }

    /// Drop reputation entries that have not been touched within `max_age`.
    pub fn prune_stale(&mut self, max_age: Duration) {
        let now = Instant::now();
        self.peers.retain(|_, s| now.duration_since(s.last_seen) < max_age);
    }

    /// Persist reputation counters to JSON.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let data: Vec<(PeerId, (u32, u32, u64))> =
            self.peers.iter().map(|(p, s)| (*p, (s.completed, s.failed, s.bytes_served))).collect();

        let json = serde_json::to_string_pretty(&data)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load reputation counters from JSON, or return an empty tracker if missing.
    pub fn load(path: &Path, ban_threshold: u32) -> Result<Self, std::io::Error> {
        if !path.exists() {
            return Ok(Self::new(ban_threshold));
        }

        let json = std::fs::read_to_string(path)?;
        let data: Vec<(PeerId, (u32, u32, u64))> = serde_json::from_str(&json)?;

        let mut tracker = Self::new(ban_threshold);
        for (peer, (completed, failed, bytes_served)) in data {
            tracker.peers.insert(
                peer,
                PeerScore { completed, failed, bytes_served, last_seen: Instant::now() },
            );
        }
        Ok(tracker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_entry_has_reasonable_score() {
        let tracker = ReputationTracker::new(5);
        let peer = PeerId::random();
        let s = tracker.score(&peer);
        assert!((s - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_success_increases_score() {
        let mut tracker = ReputationTracker::new(5);
        let peer = PeerId::random();
        tracker.record_success(peer, 1024);
        let s = tracker.score(&peer);
        assert!(s > 0.6);
    }

    #[test]
    fn test_repeated_failures_bans() {
        let mut tracker = ReputationTracker::new(3);
        let peer = PeerId::random();
        assert!(!tracker.is_banned(&peer));
        tracker.record_failure(peer);
        tracker.record_failure(peer);
        assert!(!tracker.is_banned(&peer));
        tracker.record_failure(peer);
        assert!(tracker.is_banned(&peer));
    }

    #[test]
    fn test_sort_by_score_prefers_reliable_peers() {
        let mut tracker = ReputationTracker::new(5);
        let good = PeerId::random();
        let bad = PeerId::random();
        tracker.record_success(good, 1000);
        tracker.record_success(good, 2000);
        tracker.record_failure(bad);
        tracker.record_failure(bad);

        let mut peers = vec![bad, good];
        tracker.sort_by_score(&mut peers);
        assert_eq!(peers[0], good);
        assert_eq!(peers[1], bad);
    }

    #[test]
    fn test_best_peers_filters_banned() {
        let mut tracker = ReputationTracker::new(2);
        let good = PeerId::random();
        let banned = PeerId::random();

        tracker.record_success(good, 1000);
        tracker.record_failure(banned);
        tracker.record_failure(banned);

        let result = tracker.best_peers(&[good, banned], 5);
        assert_eq!(result, vec![good]);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("reputation.json");

        let peer = PeerId::random();
        let mut tracker = ReputationTracker::new(5);
        tracker.record_success(peer, 4096);
        tracker.record_failure(peer);
        tracker.save(&path).unwrap();

        let loaded = ReputationTracker::load(&path, 5).unwrap();
        let entries = loaded.peer_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, peer);
        assert_eq!(entries[0].1.completed, 1);
        assert_eq!(entries[0].1.failed, 1);
        assert_eq!(entries[0].1.bytes_served, 4096);
    }
}
