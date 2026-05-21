# Swarm Block Protocol

## Overview

A custom block exchange protocol built on libp2p's `request-response`
framework. Nars (compressed store item archives) are split into fixed-size
blocks, distributed across peers, and downloaded in parallel.

## Why Not BitTorrent v2

BTv2 infohash = `SHA-256(merkle_root(piece_hashes) || rest_of_info_dict)`
nar-SHA-256 = `SHA-256(serialized_nar_bytes)`

These are mathematically different constructs. BTv2 cannot be aligned with
Guix's content-addressing scheme without creating a parallel namespace.

A custom protocol is:
- Directly aligned with nar-SHA-256 (key = file hash)
- Single-file (nars are always single compressed archives)
- No BTv2 spec overhead (merkle trees, bencode, hybrid handshake, choking)

## Block Layout

| Parameter | Default | Description |
|-----------|---------|-------------|
| BLOCK_SIZE | 256 KiB | Size of each block except possibly the last |
| MAX_BLOCKS | ~48,000 | ceil(200 MiB / 256 KiB) — realistic upper bound |
| MAX_BATCH | 8 | Blocks requested per peer per batch |

Block 0: bytes 0..BLOCK_SIZE-1
Block 1: bytes BLOCK_SIZE..2*BLOCK_SIZE-1
...
Block N-1: bytes (N-1)*BLOCK_SIZE..filesize-1 (last block, can be smaller)

Block count = ceil(nar_size / BLOCK_SIZE)

## Hashing

Per-block: `block_hash[i] = SHA-256(block_data[i])`
Block hash list is serialized and sent in the handshake.

Final verification: `SHA-256(block_0 || block_1 || ... || block_N-1) == nar-SHA-256`

This final check validates complete data integrity. Combined with Guix-compatible
narinfo signature verification (SPKI/libgcrypt against ACL), the trust chain is:
1. narinfo: signed by authorized key → nar-SHA-256 is authentic
2. Blocks reassembled → SHA-256 matches nar-SHA-256 → data is authentic
3. Untrusted peers can serve blocks safely — tampered data fails the hash check

## Wire Protocol

Protocol name: `"/guix/substitute/0.1.0"`

All messages use libp2p `request-response` with the built-in CBOR codec. Each
request and response is a serde enum.

### Message Types

```rust
enum BlockRequest {
    Handshake {
        nar_hash: Vec<u8>,
    },
    GetBlocks {
        nar_hash: Vec<u8>,
        indices: Vec<u32>,
    },
}

enum BlockResponse {
    HandshakeReply {
        blocks_available: Vec<u32>,  // indices this peer can serve
        block_count: u32,
        block_size: u32,
        block_hashes: Vec<Vec<u8>>,  // SHA-256 of each block
    },
    Blocks {
        data: Vec<BlockData>,
    },
    Error {
        message: String,
    },
}

struct BlockData {
    index: u32,
    data: Vec<u8>,
}
```

### Handshake Flow

```
Downloader                        Peer
  │                                 │
  │  HANDSHAKE {                    │
  │    nar_hash,                    │
  │    blocks_available (empty)     │  "I need this nar, I have no blocks yet"
  │  }                              │
  │ ──────────────────────────────> │
  │                                 │
  │  HANDSHAKE_REPLY {              │
  │    blocks_available,            │  "I have blocks 0..N"
  │    block_count, block_size,     │
  │    block_hashes                 │
  │  }                              │
  │ <────────────────────────────── │
  │                                 │
  │  GET_BLOCKS { nar_hash, indices: [17, 42] }
  │                                 │  "Send me blocks 17 and 42"
  │ ──────────────────────────────> │
  │                                 │
  │  BLOCKS { data: [(17, bytes)]} │  Peer sends blocks
  │ <────────────────────────────── │
  │                                 │
  │  ... repeat for remaining ...   │
```

### Peer Requirements

A peer serving blocks MUST:
- Respond to HANDSHAKE within 10 seconds
- Serve at least the blocks claimed in HANDSHAKE_REPLY
- Use the `nar_hash` carried by each GET_BLOCKS request to select the correct local nar
- Respond to each GET_BLOCKS request within 30 seconds
- Not require choking/unchoking (free seeding model)

A peer downloading SHOULD:
- Not request more than 8 blocks per GET_BLOCKS request
- Not send more than 4 concurrent requests per peer
- Verify SHA-256 of each received block before requesting more

## Downloader Algorithm

### State

```rust
struct NarDownloader {
    nar_hash: [u8; 32],
    nar_size: u64,
    block_count: u32,
    block_size: u32,
    block_hashes: Vec<[u8; 32]>,           // from handshake, verified by final check
    block_states: Vec<BlockFetchState>,       // pending, in-flight, or complete
    peers: HashMap<PeerId, PeerState>,       // connected peers
    received_blocks: HashMap<u32, Vec<u8>>,  // downloaded blocks awaiting final verify
}
```

### Algorithm

1. **Initialize**: from narinfo, get nar_hash and nar_size for the raw
   single-item NAR byte stream. Compute block count.
2. **Get providers**: `kad.get_providers(nar_hash)` → deduplicated burst of
   `PeerId`s for the matching NAR hash
3. **Connect**: dial each provider, establish request-response channel
4. **Handshake**: send HANDSHAKE to each peer. Collect:
   - `blocks_available` indices per peer
   - `block_hashes` from first peer (verify all peers match or fall back to HTTP)
5. **Schedule**: dynamic multi-peer block selection
   - Track each block as pending, in-flight, or complete
   - Assign pending blocks to the least-loaded peer that advertises the block
   - Keep up to `max_in_flight_blocks_per_peer` requests active per peer
6. **Download loop**:
   - For each peer, if peer has blocks we need AND peer has < 4 outstanding requests:
     - Select pending blocks this peer has
     - Send GET_BLOCKS with `nar_hash` and block indices
   - When BLOCKS response arrives:
     - Verify each block: SHA-256(block) == block_hash[index]
     - On success: store block and mark it complete
     - On failure or timeout: penalize the peer and re-queue blocks
   - Pipe new requests immediately (don't wait for batch completion)
7. **Completion**: when needed_blocks is empty
   - Join all blocks in order: block_0 || block_1 || ... || block_N-1
   - Final verify: SHA-256(full_nar) == nar-SHA-256
   - On success: write to dest path, reply "success" to daemon
   - On failure: log error, fall back to HTTP download
8. **Stall detection**: if no progress for 30 seconds → abort swarm, HTTP fallback

## Timeouts and Retries

| Operation | Timeout | Retries | Description |
|-----------|---------|---------|-------------|
| Dial peer | 10s | 2 | Connect to peer for block exchange |
| HANDSHAKE | 15s | 2 retries | Initial handshake with peer |
| GET_BLOCKS | stall timeout | until overall timeout | Download specific blocks |
| Overall download | configurable | 0 | Stall detection triggers HTTP fallback |

Retry strategy:
- Failed request-response sends requeue the affected peer's in-flight blocks
  immediately.
- Timed-out in-flight blocks are requeued before the stall guard can end an
  otherwise recoverable download.
- Different peer: retry immediately if another handshake peer advertises the
  block.
- Repeated peer failures are tracked in per-download state and long-lived peer
  reputation/backoff.

## Peer Reputation

```rust
struct PeerReputation {
    completed_downloads: u32,
    failed_downloads: u32,
    total_bytes_served: u64,
    average_response_time: Duration,
    last_seen: Instant,
}
```

Score = `completed / (completed + failed + 1)` with time-decay exponential weighting.

Used for:
- Provider selection: prefer peers with high reputation
- Blacklisting: automatic exclusion after repeated failures

## Message Serialization

Using libp2p's `request_response::cbor::Behaviour<BlockRequest,
BlockResponse>`. `BlockData.data` uses `serde_bytes` so raw block bytes are
encoded as a CBOR byte string.

## Performance Expectations

For a 100 MB nar:
- Blocks: 400 blocks of 256 KiB
- With 5 peers, each averaging 10 MB/s → theoretical 50 MB/s aggregate
- Realistic with 10 Mbps peers: 5 MB/s aggregate → 20s for download
- HTTP fallback from ci.guix.gnu.org: 50-100 MB/s → 1-2s

For small nars (< 10 MB):
- Blocks: < 40
- Swarm overhead (handshakes, connections) may exceed HTTP time
- Solution: threshold — download via HTTP directly if nar < 10 MB

For cold start (DHT lookup + peer discovery):
- DHT lookup: 0.5-2s
- Peer handshakes: 0.1-0.5s per peer
- Total pre-download: 1-3s for 8 peers
