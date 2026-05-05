# Phase 3: Swarm Downloader

## Prerequisites
Phase 2 is complete: daemon protocol works, DHT bootstrap works, HTTP narinfo fetch works.

## Goal
Implement the custom block exchange protocol over libp2p request-response.
Enable parallel block downloading from multiple DHT-discovered peers.
Full block-level and nar-level verification.

## Task Overview

Build the core of the p2p system: a swarm downloader that:
1. Takes a nar hash (from the "substitute" daemon command)
2. Queries the DHT for peers that have this nar
3. Connects to those peers, negotiates block availability
4. Downloads blocks in parallel using rarest-first selection
5. Verifies SHA-256 per block and for the full nar
6. Writes the complete nar to dest and reports success

## Tasks

### 1. src/swarm/block.rs

```rust
pub const DEFAULT_BLOCK_SIZE: usize = 262144; // 256 KiB

/// Compute block hashes for a nar file.
/// Returns (block_count, block_size, block_hashes).
/// The block_hashes are deterministic given the file.
pub fn compute_block_hashes(file_path: &Path, block_size: usize) -> io::Result<BlockInfo>;

/// Compute block layout from file size.
pub fn block_layout(file_size: u64, block_size: usize) -> (u32, usize);

/// Verify a full nar against an expected SHA-256 hash.
pub fn verify_nar_hash(file_path: &Path, expected_hash: &str) -> bool;

pub struct BlockInfo {
    pub block_count: u32,
    pub block_size: usize,
    pub block_hashes: Vec<[u8; 32]>,
}
```

Implementation details:
- `compute_block_hashes`: read the file in blocks, SHA-256 each, collect
- `block_layout`: `block_count = ceil_div(file_size, block_size)`
- `verify_nar_hash`: read entire file, SHA-256, compare to expected hex string
- Add unit tests for all functions with small known inputs

### 2. src/swarm/codec.rs

Implement `libp2p::request_response::Codec` for the block exchange protocol.

```rust
/// Protocol name registered with libp2p.
pub const PROTOCOL_NAME: &str = "/guix/substitute/0.1.0";

pub struct BlockExchangeCodec {
    max_request_size: usize,   // default 512 bytes (headers + indices)
    max_response_size: usize,  // default 2 MiB (8 * 256 KiB)
}

/// Request types from downloader to peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockRequest {
    Handshake {
        nar_hash: [u8; 32],
        /// Which blocks the downloader already has.
        have_blocks: Vec<u32>,  // sparse — only list blocks we have
    },
    GetBlocks {
        indices: Vec<u32>,  // 1..8 indices
    },
}

/// Response types from peer to downloader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockResponse {
    HandshakeReply {
        /// Which blocks this peer has.
        have_blocks: Vec<u32>,  // sparse — list of available block indices
        block_count: u32,
        block_size: u32,
        block_hashes: Vec<[u8; 32]>,
    },
    Blocks {
        /// (index, raw bytes) pairs
        data: Vec<BlockData>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockData {
    pub index: u32,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}
```

Implement `RequestResponseCodec` trait:
- `read_request` / `write_request`: read/write `BlockRequest` with length prefix
- `read_response` / `write_response`: read/write `BlockResponse` with length prefix
- Use CBOR via serde_cbor or stick with JSON for simplicity in MVP
- Max sizes prevent memory exhaustion from malformed peers

### 3. src/swarm/downloader.rs

The main download orchestrator.

```rust
pub struct SwarmDownloader {
    nar_hash: [u8; 32],
    nar_size: u64,
    block_count: u32,
    block_size: usize,
    block_hashes: Vec<[u8; 32]>,
    needed_blocks: HashSet<u32>,
    completed_blocks: HashMap<u32, Vec<u8>>,
    peer_states: HashMap<PeerId, PeerDownloadState>,
    config: SwarmConfig,
    start_time: Instant,
    last_progress: Instant,
}

pub struct SwarmConfig {
    pub max_peers: usize,           // 8
    pub max_requests_per_peer: usize, // 4
    pub blocks_per_request: usize,  // 8
    pub stall_timeout_secs: u64,    // 30
}

struct PeerDownloadState {
    blocks_available: HashSet<u32>,
    outstanding_requests: usize,
}
```

**New download:**
```rust
pub async fn download_nar(
    swarm: &mut Swarm<GuixP2PBehaviour>,
    config: &Config,
    nar_hash: &str,
    nar_size: u64,
    dest_path: &Path,
    providers: Vec<PeerId>,
) -> Result<(), SwarmError>;
```

Flow:
1. Compute block layout from nar_size
2. Initiate handshakes with all providers (up to max_peers)
3. Send `BlockRequest::Handshake` to each
4. Collect `BlockResponse::HandshakeReply` from each
5. Verify block_hashes: if multiple peers reply, they must agree
6. Build `needed_blocks = {0..block_count}`
7. Enter download loop:
   a. Compute block rarity (how many connected peers have each needed block)
   b. Sort needed blocks by rarity (ascending = rarest first)
   c. For each peer with available capacity:
      - Find up to `blocks_per_request` rarest blocks this peer has
      - Send `BlockRequest::GetBlocks { indices }`
   d. When `BlockResponse::Blocks { data }` arrives:
      - For each block: verify SHA-256(block) == block_hashes[index]
      - Store block, remove from needed_blocks
      - Log progress
      - Immediately schedule next request from this peer
   e. Check stall: if `last_progress` > stall_timeout → return StallError (triggers HTTP fallback)
   f. If `needed_blocks` is empty → exit loop
8. Join blocks in order: block_0 || block_1 || ... || block_N-1
9. Write to dest_path
10. Verify SHA-256(full_nar) == nar_hash from narinfo
11. Return Ok on success

### 4. src/behaviour.rs — Update message types

Replace the empty `SwarmRequest` / `SwarmResponse` stubs with `BlockRequest` and `BlockResponse` from the codec.

Update the behaviour definition:
```rust
#[derive(NetworkBehaviour)]
pub struct GuixP2PBehaviour {
    pub kad: Kademlia<MemoryStore>,
    pub block_exchange: request_response::Behaviour<BlockExchangeCodec>,
    pub mdns: Mdns,
    pub identify: Identify,
}
```

### 5. src/daemon.rs — Wire substitute mode

Update `handle_substitute()`:
- Extract hash part from store path
- Fetch narinfo from HTTP to get nar_size and nar_hash
- Call `dht::get_providers()` to get peer list
- If peers found (≥ 3):
  - Call `swarm::downloader::download_nar()` with those peers
  - On success: write `"success sha256:... <nar_size>"` to fd 4
  - On failure (stall/error): fall through to HTTP fallback
- If peers not found (< 3):
  - Call `fallback::download_nar()` via HTTP
  - Write result to fd 4

### 6. src/main.rs — Event loop update

Update the swarm event loop to handle block exchange responses:
```rust
loop {
    tokio::select! {
        event = swarm.select_next_some() => match event {
            SwarmEvent::Behaviour(GuixP2PEvent::BlockExchange(
                request_response::Event::Message { peer, message }
            )) => match message {
                request_response::Message::Request { request_id, request, channel } => {
                    // Peer is requesting blocks from us.
                    // Spawn a task to handle the request (read blocks from disk, reply).
                    tokio::spawn(handle_block_request(peer, request_id, request, channel));
                }
                request_response::Message::Response { request_id, response } => {
                    // Response to our request. Forward to the downloader via channel.
                    // (The downloader uses oneshot channels to bridge async events.)
                }
            },
            // ... kad, mdns, identify events ...
            _ => {}
        },
    }
}
```

### 7. tests/block.rs
- Test block_layout with various sizes (exact multiple, remainder, smaller than block)
- Test compute_block_hashes with a small known file
- Test block join + full verify roundtrip (split → modify one block → verify fails)

### 8. tests/integration.rs
- Create a test nar file with known content
- Spin up 2 local libp2p nodes in test
- Node A publishes the nar to DHT (kad.provide)
- Node B queries DHT, finds Node A, downloads blocks
- Verify the downloaded file matches original
- Use `tracing-test` to capture and assert on log output

## Deliverables
- Full block exchange protocol: handshake, request, receive, verify
- Parallel download from up to 8 peers with rarest-first scheduling
- Per-block and full-nar verification
- Integration test proves end-to-end: publish → discover → download → verify

## Verification
- `cargo fmt` passes
- `cargo clippy --all-targets --all-features -- -D warnings` passes
- `cargo test` passes (including integration tests)
- Manual test: run two instances, have one publish a nar, the other download it via DHT
