use std::{
    cmp::Reverse,
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use libp2p::{Multiaddr, PeerId, multiaddr::Protocol};
use serde::{Deserialize, Serialize};

const PEER_STORE_FILE: &str = "peers.json";
const MAX_PEER_AGE_SECS: u64 = 30 * 24 * 60 * 60;

/// Disk-backed list of recently reachable peers.
#[derive(Debug)]
pub struct PeerStore {
    path: PathBuf,
    max_entries: usize,
    entries: Vec<PeerStoreEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PeerStoreEntry {
    peer_id: String,
    address: String,
    last_seen: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PeerStoreFile {
    peers: Vec<PeerStoreEntry>,
}

impl PeerStore {
    /// Load the peer store from `<cache_dir>/peers.json`.
    pub fn load(cache_dir: &Path, max_entries: usize) -> Self {
        let path = cache_dir.join(PEER_STORE_FILE);
        let mut store = match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<PeerStoreFile>(&content) {
                Ok(file) => PeerStore { path, max_entries, entries: file.peers },
                Err(e) => {
                    tracing::warn!("failed to parse peer store {}: {}", path.display(), e);
                    PeerStore { path, max_entries, entries: Vec::new() }
                },
            },
            Err(_) => PeerStore { path, max_entries, entries: Vec::new() },
        };
        store.prune();
        store
    }

    /// Return stored peers as full `/p2p/<peer-id>` multiaddrs.
    pub fn bootstrap_peers(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|entry| entry.full_multiaddr().map(|addr| addr.to_string()))
            .collect()
    }

    /// Record a reachable address for a peer.
    pub fn record_address(&mut self, peer: PeerId, address: &Multiaddr) {
        let Some(address) = clean_address(address) else {
            return;
        };
        let now = now_unix();
        let peer_id = peer.to_string();

        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.peer_id == peer_id && entry.address == address.to_string())
        {
            entry.last_seen = now;
        } else {
            self.entries.push(PeerStoreEntry {
                peer_id,
                address: address.to_string(),
                last_seen: now,
            });
        }

        self.prune();
        if let Err(e) = self.save() {
            tracing::warn!("failed to save peer store {}: {}", self.path.display(), e);
        }
    }

    fn prune(&mut self) {
        let min_seen = now_unix().saturating_sub(MAX_PEER_AGE_SECS);
        let mut seen = HashSet::new();
        self.entries.retain(|entry| {
            if entry.last_seen < min_seen || entry.full_multiaddr().is_none() {
                return false;
            }
            seen.insert((entry.peer_id.clone(), entry.address.clone()))
        });
        self.entries.sort_by_key(|entry| Reverse(entry.last_seen));
        self.entries.truncate(self.max_entries);
    }

    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(&PeerStoreFile { peers: self.entries.clone() })?;
        fs::write(&self.path, content)?;
        Ok(())
    }
}

impl PeerStoreEntry {
    fn full_multiaddr(&self) -> Option<Multiaddr> {
        let peer_id = self.peer_id.parse::<PeerId>().ok()?;
        let address = self.address.parse::<Multiaddr>().ok()?;
        clean_address(&address).map(|addr| addr.with(Protocol::P2p(peer_id)))
    }
}

fn clean_address(address: &Multiaddr) -> Option<Multiaddr> {
    let mut addr = address.clone();
    if matches!(addr.iter().last(), Some(Protocol::P2p(_))) {
        addr.pop();
    }
    if is_unspecified(&addr) {
        return None;
    }
    Some(addr)
}

fn is_unspecified(address: &Multiaddr) -> bool {
    address.iter().any(|protocol| match protocol {
        Protocol::Ip4(ip) => ip.is_unspecified(),
        Protocol::Ip6(ip) => ip.is_unspecified(),
        _ => false,
    })
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// Build dialable addresses users can share with other peers.
pub fn shareable_addresses(external_addresses: &[String], peer_id: &str) -> Vec<String> {
    let Ok(peer_id) = peer_id.parse::<PeerId>() else {
        return Vec::new();
    };

    external_addresses
        .iter()
        .filter_map(|address| {
            let addr = address.parse::<Multiaddr>().ok()?;
            clean_address(&addr).map(|addr| addr.with(Protocol::P2p(peer_id)).to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shareable_addresses_append_peer_id() {
        let peer = PeerId::random();
        let addresses = shareable_addresses(
            &["/dns4/node.example.org/udp/6881/quic-v1".to_string()],
            &peer.to_string(),
        );

        assert_eq!(addresses, vec![format!("/dns4/node.example.org/udp/6881/quic-v1/p2p/{peer}")]);
    }

    #[test]
    fn shareable_addresses_skip_unspecified_bind_addresses() {
        let peer = PeerId::random();
        let addresses =
            shareable_addresses(&["/ip4/0.0.0.0/udp/6881/quic-v1".to_string()], &peer.to_string());

        assert!(addresses.is_empty());
    }

    #[test]
    fn peer_store_round_trips_full_multiaddr() {
        let tmp = tempfile::TempDir::new().unwrap();
        let peer = PeerId::random();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/6881".parse().unwrap();
        let mut store = PeerStore::load(tmp.path(), 8);

        store.record_address(peer, &addr);
        let loaded = PeerStore::load(tmp.path(), 8);

        assert_eq!(loaded.bootstrap_peers(), vec![format!("{addr}/p2p/{peer}")]);
    }

    #[test]
    fn peer_store_prunes_invalid_and_deduplicates_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let peer = PeerId::random();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/6881".parse().unwrap();
        let mut store = PeerStore::load(tmp.path(), 8);

        store.record_address(peer, &addr);
        store.record_address(peer, &addr);

        assert_eq!(store.bootstrap_peers().len(), 1);
    }
}
