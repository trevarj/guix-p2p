use sha2::{Digest, Sha256};

pub const DEFAULT_BLOCK_SIZE: usize = 262144;

#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub block_count: u32,
    pub block_size: usize,
    pub last_block_size: usize,
    pub block_hashes: Vec<[u8; 32]>,
}

impl BlockInfo {
    pub fn from_file_size(file_size: u64, block_size: usize) -> Self {
        assert!(block_size > 0);

        let effective = (file_size as usize).max(1);
        let bs = block_size.min(effective);
        let full_blocks = effective / bs;
        let remainder = effective % bs;
        let block_count = if remainder > 0 { full_blocks + 1 } else { full_blocks } as u32;

        BlockInfo {
            block_count,
            block_size: bs,
            last_block_size: if remainder > 0 { remainder } else { bs },
            block_hashes: Vec::with_capacity(block_count as usize),
        }
    }

    pub fn with_hashes(mut self, data: &[u8]) -> Self {
        self.block_hashes = compute_block_hashes(data, self.block_size);
        self
    }

    pub fn set_hashes(&mut self, hashes: Vec<[u8; 32]>) {
        self.block_hashes = hashes;
    }
}

pub fn compute_block_hashes(data: &[u8], block_size: usize) -> Vec<[u8; 32]> {
    data.chunks(block_size)
        .map(|chunk| {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&Sha256::digest(chunk));
            arr
        })
        .collect()
}

pub fn verify_nar(data: &[u8], expected_hex_hash: &str) -> bool {
    let hash = Sha256::digest(data);
    let got = format!("{:x}", hash);
    let expected = expected_hex_hash.strip_prefix("sha256:").unwrap_or(expected_hex_hash);
    got == expected || format!("sha256:{}", got) == expected_hex_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_info_single() {
        let info = BlockInfo::from_file_size(262144, DEFAULT_BLOCK_SIZE);
        assert_eq!(info.block_count, 1);
    }

    #[test]
    fn test_block_info_multi() {
        let info = BlockInfo::from_file_size(524288, 262144);
        assert_eq!(info.block_count, 2);
    }
}
