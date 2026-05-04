use serde::{Deserialize, Serialize};

#[allow(dead_code)]
pub const PROTOCOL_NAME: &str = "/guix/substitute/0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockData {
    pub index: u32,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockRequest {
    Handshake { nar_hash: Vec<u8> },
    GetBlocks { indices: Vec<u32> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockResponse {
    HandshakeReply {
        blocks_available: Vec<u32>,
        block_count: u32,
        block_size: u32,
        block_hashes: Vec<Vec<u8>>,
    },
    Blocks {
        data: Vec<BlockData>,
    },
    Error {
        message: String,
    },
}
