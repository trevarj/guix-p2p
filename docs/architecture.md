# Architecture: guix-p2p-substitute

## Overview

`guix-p2p-substitute` is a standalone Rust binary that speaks the guix-daemon's
existing substituter pipe protocol, but sources binary substitutes (nars) from a
libp2p-powered Kademlia DHT + custom block-swarm network instead of (or
alongside) HTTP.

Zero changes to the guix-daemon. Minimal 4-line Guile wrapper in `guix
substitute` gated by an environment variable.

## Data Flow

```
guix-daemon
  │ spawns "guix substitute --query" or "--substitute"
  │ stdin: "have /gnu/store/...", "info /gnu/store/...", "substitute /gnu/store/... /tmp/dest"
  │ fd 4:  reads "success sha256:... 12345" or "not-found" or "hash-mismatch ..."
  ▼
guix substitute (Guile, 4-line patch)
  │ if $GUIX_USE_P2P=1 → exec guix-p2p-substitute
  ▼
guix-p2p-substitute (Rust, libp2p)
  │
  ├─► Daemon Protocol Layer (stdin parser, fd 4 reply writer)
  │     "have" → DHT check if peers exist for nar hash
  │     "info" → fetch narinfo (HTTP or local cache) + return metadata
   │     "substitute" → swarm download or reply not-found → fd 4 reply
  │
  ├─► libp2p Kad DHT (QUIC transport, SHA-256 key = nar hash)
  │     get_providers(nar_hash) → list of PeerIds
  │     start_providing(nar_hash) → announce availability
  │     Bootstrap from community-maintained seed nodes
  │
  ├─► Swarm Downloader (libp2p request-response streams)
  │     Handshake: nar_hash + block availability bitfield
  │     Request: up to 8 block indices per batch
   │     Round-robin block assignment across connected peers
  │     Per-block SHA-256 verification
  │     Final nar-SHA-256 verification against narinfo
  │
   └─► HTTP Narinfo Client (reqwest)
         Narinfo fetch from official substitute URLs
         Ed25519 signature verification against /etc/guix/acl
         NarinfoCache with 60s TTL (shared via std::sync::Mutex)
```

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | tokio async runtime, mature crypto ecosystem, libp2p crate |
| DHT | libp2p-kad, SHA-256 keys | Native alignment with nar-SHA-256; battle-tested implementation |
| Swarm | Custom nar block protocol | BTv2 infohash is mathematically incompatible with nar-SHA-256 |
| Transport | QUIC (libp2p-quic) + TCP fallback | Multiplexed, performant, NAT-friendly |
| NAT traversal | Built into libp2p (autonat/relay/dcutr), deferred post-MVP | Significant complexity; initial users need open ports or IPv6 |
| Daemon integration | Env var gating in existing `guix substitute` | Zero daemon C++ changes; 4-line Guile patch |
| Narinfos | HTTP fetch from official substitute URLs | Tiny (<500 bytes); existing trust chain unchanged |
| Nars | DHT + swarm; not-found replies let guix-daemon chain to HTTP substituters | Heavy payload; distributed across peers for P2P |
| Distribution | External project, crates.io for development, Guix channel for packaging | Not targeting upstream Guix inclusion (would need pure Guile) |

## Why Not BitTorrent v2

BTv2's infohash is `SHA-256(merkle_root(piece_hashes) || rest_of_info_dict)`,
which is fundamentally different from nar-SHA-256 = `SHA-256(serialized_nar_bytes)`.
They cannot be aligned. A custom nar-specific block protocol is:

- Simpler to implement (no BTv2 spec overhead: merkle trees, bencode metainfo,
  hybrid v1/v2 handshake, choke algorithms)
- Directly aligned with existing Guix trust model (narinfo signatures)
- No external BT daemon dependency
- Single-file nar simplifies the wire protocol significantly

## Libp2p Behaviour

```rust
#[derive(NetworkBehaviour)]
struct GuixP2PBehaviour {
    kad: Kademlia<MemoryStore>,  // DHT: provide/get_providers for nar hashes
    block_exchange: RequestResponse<BlockCodec>,  // Custom block transfer protocol
    mdns: Mdns,                  // LAN peer discovery (free with libp2p)
    identify: Identify,          // Protocol versioning, agent info
}
```

## DHT Flow

```
1. "have /gnu/store/abc...-foo /gnu/store/def...-bar"
2. For each path, extract 32-char hash part
3. kad.get_providers(nar_hash) → Vec<PeerId>
4. If peers found → include path in reply to daemon
5. If none → path excluded (daemon falls through to other substituters or builds)

6. "substitute /gnu/store/abc...-foo /tmp/dest"
7. kad.get_providers(nar_hash) → Vec<PeerId>
8. Connect QUIC to each PeerId, establish block_exchange streams
9. Download blocks in parallel, verify SHA-256 per block
10. Reassemble, verify SHA-256(full nar) == narinfo NarHash
11. Write to dest, reply "success sha256:... 12345"
```

## Swarm Block Protocol

Block size: 256 KiB (configurable)
Block hash = SHA-256(block_data)
Block count = ceil(nar_size / 256KiB)

```
Client → Peer: HANDSHAKE { nar_hash: [u8; 32], blocks_available: BitVec }
Peer → Client: HANDSHAKE_REPLY { blocks_available: BitVec, block_count: u32, block_size: u32 }
Client → Peer: REQUEST { indices: [u32; 1..8] }
Peer → Client: BLOCKS { data: [(u32, Vec<u8>); 1..8] }
```

### Verification
- Per-block: SHA-256(block_data) must match known hash
- Final: SHA-256(block_0 || ... || block_N) must equal nar-SHA-256 from narinfo

### Block Selection
- Rarest-first across all connected peers
- Track availability per peer via handshake bitfields
- Prioritize blocks available from fewest peers

### Parallelism
- Max 8 concurrent peer connections per nar download
- Pipeline: request next batch before current batch completes

## Anti-Spam

Peer announcements via `kad.start_providing()` include implicit data
availability. Before trusting a provider, validate:

1. Connect to peer, request random blocks via block_exchange
2. Verify SHA-256 for each received block
3. If verification fails, blacklist peer for this nar hash

Expired records are handled by libp2p-kad's TTL-based record management.

## HTTP Narinfo Client

Narinfos are fetched from official substitute URLs and verified against ACL keys.
Nar downloads are handled by guix-daemon's substituter chaining (reply `not-found`
to let daemon fall through to HTTP substituters).

Safety thresholds:
- DHT returns < `min_providers` (3) peers → skip swarm, reply not-found
- Swarm download stalls (no new blocks for `stall_timeout_secs` (30s)) → abort, reply not-found
- Nar hash verification failed → reply not-found
- Narinfo signature verification failed → report error, no fallback (security)

Narinfo flow:
1. Check NarinfoCache (60s TTL, shared via `std::sync::Mutex`)
2. `reqwest` GET `<substitute_url>/<hash-part>.narinfo`
3. Verify Ed25519 signature against `/etc/guix/acl` public keys
4. Cache result, return parsed Narinfo

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `libp2p` | Core: swarm, transport, identity, PeerId
(features: kad, quic, tcp, dns, request-response, mdns, identify, autonat, relay) |
| `tokio` | Async runtime, channels, sync primitives |
| `tokio-stream` | UnboundedReceiverStream for swarm notification channel |
| `ed25519-dalek` | Key generation, signing, signature verification |
| `sha2` | SHA-256 (block hashes, nar verification) |
| `reqwest` | HTTP narinfo fetching from official substitute URLs |
| `serde` / `serde_json` | Protocol message serialization, config parsing |
| `serde_bytes` | Efficient byte slice serialization for protocol messages |
| `clap` | CLI argument parsing |
| `tracing` / `tracing-subscriber` | Structured logging with env-filter |
| `bitvec` | Block availability bitfields |
| `anyhow` | Application-level error handling |
| `base64` | Base64 decoding for narinfo signatures |
| `hex` | Hex encoding/decoding for hash strings |
| `futures` | Async combinators |
| `libc` | Raw fd writing for daemon protocol (fd 4) |
| `thiserror` | Library-level error types (HttpClientError, ParseError) |
| `tempfile` | Temporary files for integration tests |

## NAT Traversal (Post-MVP)

libp2p provides built-in behaviours:
- `autonat` — detect if behind NAT
- `relay` — connect via relay nodes when direct connection fails
- `dcutr` — Direct Connection Upgrade through Relay (hole-punching)

When ready, it's a behaviour mix-in and relay node infrastructure deployment.
Not a protocol rewrite.

## Guix Integration Patch

In `guix/scripts/substitute.scm`, after the `guix-substitute` entry point:

```scheme
(if (and=> (getenv "GUIX_USE_P2P")
           (cut string-ci=? <> "yes"))
    (begin
      (dup2 (fileno (current-output-port)) 4)
      (apply execlp "guix-p2p-substitute"
             "guix-p2p-substitute"
             (cdr (command-line))))
    (begin
      ;; existing substitute logic
      ))
```

Files changed in Guix:
- `guix/scripts/substitute.scm` — +4 lines, environment variable gating
