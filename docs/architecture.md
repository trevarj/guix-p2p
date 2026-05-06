# Architecture: guix-p2p

## Overview

`guix-p2p` is a standalone Rust binary that speaks the guix-daemon's
existing substituter pipe protocol, but sources binary substitutes (nars) from a
libp2p-powered Kademlia DHT + custom block-swarm network instead of (or
alongside) HTTP.

Zero changes to the guix-daemon. Minimal 4-line Guile wrapper in `guix
substitute` gated by an environment variable.

## Architecture Overview

The binary runs in two modes:
- **Daemon** (`--daemon`): Persistent process with warm libp2p swarm, listening
  on a Unix domain socket for relay connections
- **Relay** (`--query --socket PATH` or `--substitute --socket PATH`): Thin
  client that forwards stdin/fd4 through the Unix socket to the daemon,

  avoiding cold-start cost

## Data Flow

```
guix-daemon
  │ spawns "guix substitute --query" or "--substitute"
  │ stdin: "have /gnu/store/...", "info /gnu/store/...", "substitute /gnu/store/... /tmp/dest"
  │ fd 4:  reads "success sha256:... 12345" or "not-found" or "hash-mismatch ..."
  ▼
guix substitute (Guile, 4-line patch) or PATH wrapper
  │ if $GUIX_USE_P2P=1 or wrapper detects "substitute" → exec guix-p2p
  ▼
guix-p2p (Rust, libp2p)
  │
  ├─► Daemon Mode (--daemon)
  │     Unix socket listener at $XDG_CACHE_HOME/guix-p2p/guix-p2p.sock
  │     Accepts relay connections, processes query/substitute requests
  │     Warm libp2p swarm, seeds to peers
  │
  ├─► Relay Mode (--query --socket PATH / --substitute --socket PATH)
  │     Connects to daemon's Unix socket
  │     Sends mode header + forwards stdin to daemon
  │     Writes daemon replies to fd 4
  │     Near-zero startup cost (<1ms vs cold-start libp2p init)
  │
  ├─► Direct Mode (--query / --substitute without --socket)
  │     Legacy mode: initializes own libp2p swarm and processes requests
  │     Higher startup cost, kept for development/fallback
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
  │     Peer reputation scoring (time-decay, ban threshold)
  │     Connection management (retry/backoff, dead peer pruning)
  │
  ├─► Bandwidth Limiter (token-bucket, configurable caps)
  │
  └─► HTTP Narinfo Client (reqwest)
       Narinfo fetch from official substitute URLs
       Ed25519 signature verification against /etc/guix/acl
       NarinfoCache with 60s TTL (shared via Arc<Mutex>)
```

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | tokio async runtime, mature crypto ecosystem, libp2p crate |
| DHT | libp2p-kad, SHA-256 keys | Native alignment with nar-SHA-256; battle-tested implementation |
| Swarm | Custom nar block protocol | BTv2 infohash is mathematically incompatible with nar-SHA-256 |
| Transport | QUIC (libp2p-quic) + TCP fallback | Multiplexed, performant, NAT-friendly |
| NAT traversal | Built into libp2p (autonat/relay/dcutr), deferred post-MVP | Significant complexity; initial users need open ports or IPv6 |
| Daemon integration | Unix socket relay + PATH wrapper | Zero daemon C++ changes; relay gives <1ms startup |
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
    kad: Kademlia<MemoryStore>,                     // DHT: provide/get_providers for nar hashes
    block_exchange: RequestResponse<BlockCodec>,     // Custom block transfer protocol
    mdns: Mdns,                                     // LAN peer discovery (free with libp2p)
    identify: Identify,                             // Protocol versioning, agent info
}
```

Connection management, peer reputation, and bandwidth limiting are implemented
as separate modules outside the behaviour:

```rust
struct ReputationTracker { .. }     // src/reputation.rs: time-decay peer scoring
struct ConnectionManager { .. }     // src/connection.rs: retry/backoff, pruning
struct BandwidthLimiter { .. }     // src/bandwidth.rs: token-bucket rate limiting
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
| `serde` / `serde_json` | Protocol message serialization, config parsing, reputation persistence |
| `serde_bytes` | Efficient byte slice serialization for protocol messages |
| `clap` | CLI argument parsing |
| `tracing` / `tracing-subscriber` | Structured logging with env-filter |
| `bitvec` | Block availability bitfields |
| `anyhow` | Application-level error handling |
| `base64` | Base64 decoding for narinfo signatures |
| `hex` | Hex encoding/decoding for hash strings |
| `futures` | Async combinators |
| `libc` | Raw fd writing for daemon protocol (fd 4) |
| `thiserror` | Library-level error types (HttpClientError, DownloadError, ParseError) |
| `tempfile` | Temporary files for integration tests |

## Source Layout

```
src/
├── lib.rs                   # Crate root (public API for integration tests)
├── main.rs                  # CLI, swarm task, daemon/relay mode dispatch
├── behaviour.rs             # libp2p NetworkBehaviour (kad + block_exchange + mdns + identify)
├── channel.rs               # SwarmCommand / SwarmNotification enums (broadcast channel types)
├── config.rs                # Config struct (block_size, timeouts, ACL path, socket_path, etc.)
├── connection.rs            # ConnectionManager (retry/backoff, dead peer pruning, max peers)
├── daemon.rs                # stdin parser, fd 4 reply writer, swarm substitute pipeline, daemon + socket listener
├── relay.rs                 # Unix socket relay client (stdin → socket → fd 4)
├── dht.rs                   # Kad wrapper, handle_kad_event → notifications, get_providers, bootstrap
├── reputation.rs            # ReputationTracker (time-decay scoring, ban threshold, JSON persistence)
├── bandwidth.rs             # BandwidthLimiter (token-bucket, configurable caps)
├── dashboard.rs             # Web dashboard (optional, --dashboard flag)
├── http_client.rs           # Narinfo fetch (HTTP only), signature verification, cache
├── narinfo.rs               # Narinfo parser, ACL loader, Ed25519 verifier, NarinfoCache (with TTL eviction)
├── nar_store.rs             # NarStore: local nar cache, block serving, seeding via guix archive --export
├── identity.rs              # Ed25519 keypair gen/persistence
└── swarm/
    ├── mod.rs
    ├── block.rs             # BlockInfo (block count, size, hashes)
    ├── codec.rs             # Request/response message types (BlockRequest/BlockResponse)
    └── downloader.rs        # ActiveDownload state machine, peer pool, block verification
```

## NAT Traversal (Post-MVP)

libp2p provides built-in behaviours:
- `autonat` — detect if behind NAT
- `relay` — connect via relay nodes when direct connection fails
- `dcutr` — Direct Connection Upgrade through Relay (hole-punching)

When ready, it's a behaviour mix-in and relay node infrastructure deployment.
Not a protocol rewrite.

## Activation

The binary accepts `--query` / `--substitute` / `--daemon` as top-level flags
(matching guix-daemon's invocation of `guix substitute --query`). The `--socket`
flag selects relay mode for `--query` and `--substitute`.

### Daemon + Relay Architecture

The recommended deployment uses a persistent daemon and thin relay clients:

1. **Daemon** (`guix-p2p --daemon`): Listens on a Unix domain socket
   (`$XDG_CACHE_HOME/guix-p2p/guix-p2p.sock` by default). Keeps the libp2p
   swarm warm, serves block requests, processes queries and substitutes from
   relay connections.

2. **Relay** (`guix-p2p --query --socket PATH`): Connects to the daemon's Unix
   socket, sends mode header (`mode: query\n`), then forwards stdin lines and
   writes daemon replies to fd 4. Startup is <1ms since no libp2p
   initialization is needed.

Each relay connection sends a mode header and then streams daemon protocol
commands. The daemon processes each connection independently, subscribing to
the swarm's broadcast notification channel for that connection.

### PATH Wrapper (`scripts/guix-wrapper.sh`)

A thin shell script placed earlier in `$PATH` than the real `guix` binary.
Detects whether the daemon's socket is available and uses relay mode when
possible, falling back to direct invocation otherwise:

```
guix-daemon invokes "guix substitute --query"
  → wrapper intercepts "substitute"
  → if socket exists: exec guix-p2p --query --socket $SOCKET
  → else: exec real guix substitute --query

guix-daemon invokes "guix substitute --substitute"
  → wrapper intercepts "substitute"
  → if socket exists: exec guix-p2p --substitute --socket $SOCKET
  → else: exec real guix substitute --substitute

user invokes "guix build/install/system/..."
  → wrapper passes through to real guix unchanged
```

This works for ALL guix commands (build, install, pull, system reconfigure,
home reconfigure, shell) because they all go through the same daemon
substitute protocol.

### Shepherd Service (Daemon Mode)

```scheme
(define guix-p2p-daemon
  (make <service>
    #:provides '(guix-p2p-daemon)
    #:start (make-forkexec-constructor
             '("guix-p2p" "--daemon"
               "--listen-addr" "/ip4/0.0.0.0/udp/6881/quic-v1"
               "--cache-dir" "/var/cache/guix-p2p")
             #:log-file "/var/log/guix-p2p-daemon.log")
    #:stop  (make-kill-destructor)
    #:respawn? #t))
```

### Future: Upstream Guile Patch

If upstream merges a patch to `guix/scripts/substitute.scm`, the PATH
wrapper becomes unnecessary. The patch would detect the P2P socket and
relay through it directly.

## Seeding Strategy

Nars are seeded from a local cache directory after successful downloads or
explicit `--seed` paths. The `NarStore` (`src/nar_store.rs`) manages storage
and serving.

### Approach: Hybrid nar cache seeding

- **Post-download seeding**: After a successful swarm download, the nar is
  saved to `<cache_dir>/nar/<sha256hex>.nar` and announced in the DHT via
  `start_providing`. Future peers can download it from this node.
- **Explicit seeding**: The `--seed` flag accepts comma-separated store paths.
  Each is exported via `guix archive --export`, hashed via `guix hash -S nar
  -f hex`, and stored in the nar cache. All seeded nars are announced in the
  DHT on startup.
- **Startup scan**: On startup, `NarStore::new()` scans `<cache_dir>/nar/*.nar`
  and indexes each file by its filename stem (the hex sha256). All indexed
  nars are announced in the DHT.
- **Serving**: Incoming block requests are served from the nar store. The
  `NarStore::handle_request()` method dispatches to handshake replies (with
  block hashes) or block data reads. The old `serve_block_request()` stub has
  been replaced.

### NarStore wire-up

- `NarStore` is wrapped in `Arc<Mutex<NarStore>>` and shared between:
  - The swarm task (serves incoming block requests)
  - The daemon substitute mode (saves nars after successful downloads)
  - The daemon socket listener (same as substitute mode, for relay connections)
- After saving a nar, a `SwarmCommand::StartProviding { hash }` is sent to the
  swarm task to announce the new nar in the DHT.

### `--seed` CLI flag

```
guix-p2p --daemon --seed /gnu/store/...-foo,/gnu/store/...-bar
```

Each path is fed to `NarStore::seed_store_path()`, which runs `guix hash` and
`guix archive --export` to compute the hash and export the nar data.

## Dashboard Seeding View

The web dashboard (`--dashboard`) includes a **seeds** panel that shows all
locally-seeded nars in real time:

- **Seed list**: Each seeded nar is displayed with its hash, size, and block
  count. Clicking a row opens a detail overlay.
- **Real-time events**: `BlockServed` events stream via WebSocket showing
  which blocks are being uploaded to which peers. Served rows flash green
  momentarily.
- **Seed count** in the header bar updates as nars are seeded or auto-saved
  after downloads.
- The `/api/seeds` endpoint returns the full list of seeded nars with size,
  block count, and block size.

Dashboard events related to seeding:

| Event | Description |
|-------|-------------|
| `SeedAdded` | Emitted when a nar is added to the local store (startup seeding or post-download) |
| `BlockServed` | Emitted when blocks are served to a requesting peer (includes nar hash, peer, indices) |
