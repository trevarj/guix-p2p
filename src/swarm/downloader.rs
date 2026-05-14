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

pub struct ActiveDownload {
    pub nar_hash_hex: String,
    pub nar_size: u64,
    pub block_info: BlockInfo,
    pub dest_path: PathBuf,
    pub store_path: String,
    pub receivers: HashMap<PeerId, DownloadResult>,
    pub started: Instant,
}

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

    pub fn record_blocks(&mut self, peer: PeerId, blocks: &[(u32, Vec<u8>)]) {
        if let Some(result) = self.receivers.get_mut(&peer) {
            for (idx, data) in blocks {
                if *idx < result.blocks.len() as u32 {
                    let expected = &self.block_info.block_hashes[*idx as usize];
                    let actual = Sha256::digest(data);
                    if actual.as_slice() == expected {
                        result.blocks[*idx as usize] = Some(data.clone());
                    }
                }
            }
        }
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
