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

All messages use libp2p `request-response` with a single codec. Each message
is a typed enum serialized with serde.

### Message Types

```rust
enum SwarmMessage {
    Handshake {
        nar_hash: [u8; 32],
        blocks_available: BitVec,  // which blocks this peer has
    },
    HandshakeReply {
        blocks_available: BitVec,  // which blocks this peer has
        block_count: u32,
        block_size: u32,
        block_hashes: Vec<[u8; 32]>,  // SHA-256 of each block
    },
    Request {
        nar_hash: [u8; 32],
        indices: Vec<u32>,  // 1..8 block indices
    },
    Blocks {
        data: Vec<(u32, Vec<u8>)>,  // (index, raw bytes) for each block
    },
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
  │  REQUEST { nar_hash, indices: [17, 42] }
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
- Use the `nar_hash` carried by each REQUEST to select the correct local nar
- Respond to each REQUEST within 30 seconds
- Not require choking/unchoking (free seeding model)

A peer downloading SHOULD:
- Not request more than 8 blocks per REQUEST
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
    needed_blocks: BitVec,                   // blocks still to download
    peers: HashMap<PeerId, PeerState>,       // connected peers
    received_blocks: HashMap<u32, Vec<u8>>,  // downloaded blocks awaiting final verify
}
```

### Algorithm

1. **Initialize**: from narinfo, get nar_hash and nar_size for the raw
   single-item NAR byte stream. Compute block count.
2. **Get providers**: `kad.get_providers(nar_hash)` → Vec<PeerId>
3. **Connect**: dial each provider, establish request-response channel
4. **Handshake**: send HANDSHAKE to each peer. Collect:
   - `blocks_available` bitfield per peer
   - `block_hashes` from first peer (verify all peers match or fall back to HTTP)
5. **Schedule**: rarest-first block selection
   - Count how many connected peers have each block (from bitfields)
   - Sort needed blocks by availability count (ascending = rarest first)
   - Request rarest blocks first
6. **Download loop**:
   - For each peer, if peer has blocks we need AND peer has < 4 outstanding requests:
     - Select up to 8 rarest blocks this peer has
     - Send REQUEST with `nar_hash` and block indices
   - When BLOCKS response arrives:
     - Verify each block: SHA-256(block) == block_hash[index]
     - On success: store block, mark as received in needed_blocks
     - On failure: blacklist peer for this nar, re-queue blocks
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
| HANDSHAKE | 10s | 1 | Initial handshake with peer |
| REQUEST | 30s | 2 | Download specific blocks |
| Overall download | configurable | 0 | Stall detection triggers HTTP fallback |

Retry strategy:
- Same peer: exponential backoff (1s → 2s → 4s)
- Different peer: retry immediately if other providers exist
- After 2 failures from a peer: remove from peer pool for this nar

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
- Block assignment: assign critical (rarest) blocks to highest-reputation peers
- Blacklisting: automatic exclusion after repeated failures

## Message Serialization

Using serde with CBOR for compact binary format (alternative: prost/protobuf).

```rust
impl RequestResponseCodec for BlockExchangeCodec {
    type Protocol = StreamProtocol;
    type Request = SwarmRequest;   // HANDSHAKE or REQUEST
    type Response = SwarmResponse; // HANDSHAKE_REPLY or BLOCKS

    fn read_request<T>(&mut self, protocol: &Self::Protocol, io: &mut T) 
        -> io::Result<Self::Request>;

    fn read_response<T>(&mut self, protocol: &Self::Protocol, io: &mut T)
        -> io::Result<Self::Response>;

    fn write_request<T>(&mut self, protocol: &Self::Protocol, io: &mut T,
                        req: Self::Request) -> io::Result<()>;

    fn write_response<T>(&mut self, protocol: &Self::Protocol, io: &mut T,
                         res: Self::Response) -> io::Result<()>;
}
```

Wire format per message:
```
[4 bytes: total_message_length]
[varint: protocol_version = 1]
[1 byte: message_type]
   0 = HANDSHAKE
   1 = HANDSHAKE_REPLY
   2 = REQUEST
   3 = BLOCKS
[cbor_encoded_payload]
```

BitVec serialization: length-prefixed raw bytes (ceil(bits/8) bytes).

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
