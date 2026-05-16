use std::{collections::HashMap, path::PathBuf, time::Instant};

use libp2p::PeerId;
use sha2::{Digest, Sha256};

use crate::swarm::block::BlockInfo;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("incomplete")]
    Incomplete,
    #[error("hash mismatch")]
    HashMismatch,
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Tracks verified blocks for one active NAR download.
///
/// The scheduler in `daemon.rs` decides which peer should serve each block.
/// `ActiveDownload` owns integrity checks and final assembly: every accepted
/// block must match the per-block hash learned during handshake, and the joined
/// byte stream must match the narinfo hash before it is returned.
pub struct ActiveDownload {
    pub nar_hash_hex: String,
    pub nar_size: u64,
    pub block_info: BlockInfo,
    pub dest_path: PathBuf,
    pub store_path: String,
    pub receivers: HashMap<PeerId, DownloadResult>,
    pub started: Instant,
}

/// Verified blocks received from one peer.
///
/// Blocks are stored per peer so duplicate or malicious data from one peer does
/// not overwrite already accepted data from another peer.
pub struct DownloadResult {
    pub blocks: Vec<Option<Vec<u8>>>,
}

impl ActiveDownload {
    pub fn new(
        nar_hash_hex: String,
        nar_size: u64,
        block_info: BlockInfo,
        dest_path: PathBuf,
        store_path: String,
    ) -> Self {
        ActiveDownload {
            nar_hash_hex,
            nar_size,
            block_info,
            dest_path,
            store_path,
            receivers: HashMap::new(),
            started: Instant::now(),
        }
    }

    pub fn add_peer(&mut self, peer: PeerId) {
        self.receivers.insert(
            peer,
            DownloadResult { blocks: vec![None; self.block_info.block_count as usize] },
        );
    }

    /// Verify and store newly received blocks from a peer.
    ///
    /// Returns only the indices accepted by this call. Duplicate blocks and
    /// blocks with a bad SHA-256 digest are ignored so the scheduler can retry
    /// them with another provider.
    pub fn record_blocks(&mut self, peer: PeerId, blocks: &[(u32, Vec<u8>)]) -> Vec<u32> {
        let mut accepted = Vec::new();

        if let Some(result) = self.receivers.get_mut(&peer) {
            for (idx, data) in blocks {
                if *idx < result.blocks.len() as u32 {
                    let expected = &self.block_info.block_hashes[*idx as usize];
                    let actual = Sha256::digest(data);
                    if actual.as_slice() == expected && result.blocks[*idx as usize].is_none() {
                        result.blocks[*idx as usize] = Some(data.clone());
                        accepted.push(*idx);
                    }
                }
            }
        }

        accepted
    }

    pub fn is_complete(&self) -> bool {
        // Check if any peer has all blocks, or if we can assemble from multiple peers
        for result in self.receivers.values() {
            if result.blocks.iter().all(Option::is_some) {
                return true;
            }
        }
        // Cross-peer check: at least one copy of each block
        let count = self.block_info.block_count as usize;
        for i in 0..count {
            if !self.receivers.values().any(|r| r.blocks[i].is_some()) {
                return false;
            }
        }
        true
    }

    pub fn assemble(&self) -> Result<Vec<u8>, DownloadError> {
        let mut nar = Vec::with_capacity(self.nar_size as usize);
        let count = self.block_info.block_count as usize;

        for i in 0..count {
            let block = self.receivers.values().find_map(|r| r.blocks[i].as_ref());
            match block {
                Some(b) => nar.extend_from_slice(b),
                None => return Err(DownloadError::Incomplete),
            }
        }

        if self.nar_size > 0 && nar.len() != self.nar_size as usize {
            return Err(DownloadError::Incomplete);
        }

        let hash = Sha256::digest(&nar);
        let got = format!("{:x}", hash);
        let expected = &self.nar_hash_hex;

        if &got != expected && format!("sha256:{}", got) != *expected {
            return Err(DownloadError::HashMismatch);
        }

        Ok(nar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn download_for_blocks(blocks: &[&[u8]]) -> ActiveDownload {
        let data = blocks.concat();
        let mut info = BlockInfo::from_file_size(data.len() as u64, 4);
        info.block_hashes = blocks
            .iter()
            .map(|block| {
                let hash = Sha256::digest(block);
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&hash);
                arr
            })
            .collect();
        info.block_count = blocks.len() as u32;

        ActiveDownload::new(
            format!("{:x}", Sha256::digest(&data)),
            data.len() as u64,
            info,
            PathBuf::new(),
            String::new(),
        )
    }

    #[test]
    fn record_blocks_returns_newly_accepted_indices() {
        let peer = PeerId::random();
        let mut download = download_for_blocks(&[b"abcd", b"efgh"]);
        download.add_peer(peer);

        let accepted = download.record_blocks(peer, &[(0, b"abcd".to_vec())]);
        assert_eq!(accepted, vec![0]);

        let duplicate = download.record_blocks(peer, &[(0, b"abcd".to_vec())]);
        assert!(duplicate.is_empty());
    }

    #[test]
    fn record_blocks_rejects_hash_mismatches() {
        let peer = PeerId::random();
        let mut download = download_for_blocks(&[b"abcd"]);
        download.add_peer(peer);

        let accepted = download.record_blocks(peer, &[(0, b"wxyz".to_vec())]);
        assert!(accepted.is_empty());
        assert!(!download.is_complete());
    }
}
