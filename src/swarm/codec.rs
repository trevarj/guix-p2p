use serde::{Deserialize, Serialize};

#[allow(dead_code)]
pub const PROTOCOL_NAME: &str = "/guix/substitute/0.1.0";

/// One indexed NAR block carried in a block response.
///
/// Block payloads are serde-byte encoded so CBOR transports the bytes as a
/// compact byte string instead of as an array of integers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockData {
    pub index: u32,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

/// Request messages for the guix-p2p block exchange protocol.
///
/// A downloader first asks a provider which blocks it can serve for a NAR hash,
/// then requests concrete block indices once scheduling has assigned work to
/// that peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockRequest {
    /// Ask a provider for block availability and per-block hashes.
    Handshake { nar_hash: Vec<u8> },
    /// Fetch the listed block indices for the requested NAR.
    GetBlocks { nar_hash: Vec<u8>, indices: Vec<u32> },
}

/// Response messages for the guix-p2p block exchange protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockResponse {
    /// Provider metadata used to initialize the block scheduler.
    HandshakeReply {
        blocks_available: Vec<u32>,
        block_count: u32,
        block_size: u32,
        block_hashes: Vec<Vec<u8>>,
    },
    /// Raw block payloads for a prior `GetBlocks` request.
    Blocks { data: Vec<BlockData> },
    /// Protocol-level failure that can still be delivered over the stream.
    Error { message: String },
}
