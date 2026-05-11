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
  ├─► libp2p Kad DHT (QUIC/TCP transport, SHA-256 key = nar hash)
  │     get_providers(nar_hash) → list of PeerIds
  │     start_providing(nar_hash) → announce availability
  │     Bootstrap from community-maintained seed nodes
  │
  ├─► Swarm Downloader (libp2p request-response streams)
  │     Handshake: nar_hash + block availability bitfield
  │     Request: nar_hash + up to 8 block indices per batch
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
| Transport | QUIC (libp2p-quic) + TCP fallback | QUIC is the default listen address; TCP is enabled for restricted containers and networks where UDP is unavailable |
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

## Daemon Protocol Layer

The daemon protocol follows guix-daemon's substituter pipe protocol exactly.

### Socket relay protocol

The Unix socket between daemon and relay uses channel prefix framing:
- `fd4:<line>\n` — structured reply data (have paths, info metadata, success/not-found)
- `out:<line>\n` — trace output (`@ download-started`, `@ download-succeeded`)
- `nar:<base64-chunk>\n` / `nar-end\n` — verified NAR bytes for the current
  substitute request

The relay demuxes these: `fd4:` lines are written to fd 4, `out:` lines to
stdout (fd 1), and `nar:` chunks are buffered to a temporary NAR file. At
`nar-end`, the relay restores that NAR into the destination path from the
`substitute <store-path> <dest>` command with Guix's NAR deserializer through
`guix repl`, so the helper has the same Guix module load path as the installed
`guix` command.
Destination writes happen in the relay process spawned by `guix-daemon`, not in
the long-lived user daemon. This keeps the warm swarm architecture while
matching guix-daemon's permission model.

### Query protocol

- **have**: daemon writes `have <path1> <path2> ...\n`. Reply: each available
  path on a separate line, terminated by blank line.
- **info**: daemon writes `info <path1> ...\n`. Reply per path: store_path,
  deriver, ref_count, refs, download_size, nar_size, then blank line.
  Narinfo derivers and references are returned as full `/gnu/store/...` paths.
  In `p2p-only` mode, info is returned only when the corresponding NarHash has
  enough P2P providers.

### Substitute protocol

- **substitute**: daemon writes `substitute <store-path> <dest>\n`. Reply:
  `success sha256:<hash> <size>`, or `hash-mismatch sha256 <expected> <actual>`,
  or `not-found`.

Before the download, `@ download-started <path> <url> <size>` is written to
the trace channel. After success, `@ download-succeeded <path> <url> <size>`
is written. These match the guix-daemon build trace protocol.

### Nar hash verification

After downloading (P2P or HTTP), the nar's SHA-256 hash is verified against
the narinfo's expected `NarHash`. If they don't match:
1. The destination file is deleted
2. `hash-mismatch sha256 <expected> <actual>` is replied on fd 4
3. guix-daemon treats this as a corruption error (not a simple fallback)

## DHT Flow

```
1. "have /gnu/store/abc...-foo /gnu/store/def...-bar"
2. For each path, extract 32-char hash part
3. If policy is http-first or p2p-first:
     → include all paths in reply (we can serve via HTTP)
   If policy is p2p-only:
     → kad.get_providers(nar_hash) → Vec<PeerId>
     → include path only if peers found

4. "substitute /gnu/store/abc...-foo /tmp/dest"
5. Fetch narinfo from substitute servers, verify signature
6. If policy is http-first:
     → try HTTP nar download first
     → on failure, fall back to P2P swarm
   If policy is p2p-first:
     → try P2P swarm first
     → on failure, fall back to HTTP nar download
   If policy is p2p-only:
     → P2P swarm only, fail on no providers
7. On success: write nar to dest, save to NarStore for re-seeding, announce in DHT
8. Reply "success sha256:... <size>" or "not-found <path>"
```

## Swarm Block Protocol

Block size: 256 KiB (configurable)
Block hash = SHA-256(block_data)
Block count = ceil(nar_size / 256KiB)

```
Client → Peer: HANDSHAKE { nar_hash: [u8; 32], blocks_available: BitVec }
Peer → Client: HANDSHAKE_REPLY { blocks_available: BitVec, block_count: u32, block_size: u32 }
Client → Peer: REQUEST { nar_hash: [u8; 32], indices: [u32; 1..8] }
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
Nar downloads use the substitute policy to choose between P2P and HTTP.

### Substitute Policy

Three modes control how nars are sourced, configured via `--policy` CLI flag
or `substitute_policy` in the TOML config file:

| Mode | Behavior |
|------|----------|
| `p2p-only` | Only use P2P swarm. Fail with `not-found` if no peers. No HTTP nar download. |
| `p2p-first` | Try P2P first. Fall back to HTTP nar download if swarm fails (not enough providers, handshake failure, download error). |
| `http-first` | Try HTTP nar download first. Fall back to P2P if HTTP fails or returns 404. |

Default: `p2p-first`.

The policy also affects the `have` query:
- `http-first` and `p2p-first`: Always respond with the path (we can serve via HTTP fallback).
- `p2p-only`: Only respond if DHT providers exist for the nar hash.

### HTTP Nar Download

When the policy allows HTTP fallback, nars are downloaded from the substitute
server URLs in the narinfo. Decompression supports gzip and zstd; lzip is not
yet supported. The preference order is: zstd > gzip > none.

### Safety thresholds:
- DHT returns < `min_providers` (3) peers → skip swarm, reply not-found
- Swarm download stalls (no new blocks for `stall_timeout_secs` (30s)) → abort, reply not-found
- Nar hash verification failed → reply not-found
- Narinfo signature verification failed → report error, no fallback (security)

Narinfo flow:
1. Check NarinfoCache (60s TTL, shared via `std::sync::Mutex`)
2. `reqwest` GET `<substitute_url>/<hash-part>.narinfo`
3. Verify Guix's SPKI signature with libgcrypt against `/etc/guix/acl`
   public keys
4. Decode `NarHash: sha256:<nix-base32>` to raw SHA-256 bytes for DHT/swarm
   keys
5. Cache result, return parsed Narinfo

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `libp2p` | Core: swarm, transport, identity, PeerId
(features: kad, quic, tcp, dns, request-response, mdns, identify, autonat, relay) |
| `tokio` | Async runtime, channels, sync primitives |
| `tokio-stream` | UnboundedReceiverStream for swarm notification channel |
| `ed25519-dalek` | libp2p identity key handling and ACL key parsing |
| `libgcrypt-sys` | Guix-compatible SPKI narinfo signature verification |
| `sha2` | SHA-256 (block hashes, nar verification) |
| `reqwest` | HTTP narinfo fetching from official substitute URLs |
| `serde` / `serde_json` | Protocol message serialization, config parsing, reputation persistence |
| `toml` | TOML config file parsing |
| `flate2` | Gzip decompression for HTTP nar downloads |
| `zstd` | Zstd decompression for HTTP nar downloads |
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
├── nar_store.rs             # NarStore: local nar cache, block serving, raw single-item nar seeding
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

mDNS LAN discovery is best-effort. It is enabled when the OS permits multicast
sockets and disabled with a warning in restricted containers or sandboxes.

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

For bootstrap-node operations, use the Shepherd-first guide in
[`bootstrap-node.md`](bootstrap-node.md).

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

### TOML Config File

Settings can be persisted in `$XDG_CONFIG_HOME/guix-p2p/config.toml`
(or `~/.config/guix-p2p/config.toml`). CLI flags override file values.
See [`configuration.md`](configuration.md) for every supported key, default,
and CLI override.

```toml
substitute_policy = "p2p-first"
bootstrap_peers = "/ip4/1.2.3.4/udp/6881/quic-v1/p2p/QmPeer1,/ip4/5.6.7.8/udp/6881/quic-v1/p2p/QmPeer2"
substitute_urls = "https://bordeaux.guix.gnu.org,https://ci.guix.gnu.org"
min_providers = 3
request_timeout_secs = 30
stall_timeout_secs = 30
block_size = 262144
max_peers_per_download = 8
max_total_peers = 50
acl_path = "/etc/guix/acl"
seed_paths = ["/gnu/store/abc-foo", "/gnu/store/def-bar"]
```

### E2E Harness Flags

The `guix-p2p-e2e container-smoke` harness has its own CLI surface for local
proofs. It supports `--dashboard-bind` for VM port forwarding and `--hold` to
keep validated smoke-test dashboards running until Ctrl-C. These flags do not
change production daemon configuration.

The disposable VM path uses `container-smoke --vm-direct` so the harness runs
peer and daemon processes directly inside the writable qcow2 guest instead of
nesting `guix shell -CN`. `GUIX_P2P_E2E_HOLD=1` is the VM wrapper switch for
interactive dashboard inspection after validation.

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
  Each is hashed via `guix hash -S nar -f hex`, serialized as a raw
  single-item NAR with Guix's `(guix serialization) write-file`, and stored in
  the nar cache. All seeded nars are announced in the DHT on startup.
- **Startup scan**: On startup, `NarStore::new()` scans `<cache_dir>/nar/*.nar`
  and indexes each file by its filename stem (the hex sha256). Files whose
  bytes do not hash to the filename stem are skipped. All indexed nars are
  announced in the DHT.
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
Guix's raw NAR serializer. It intentionally does not use `guix archive
--export`, because that command writes a signed nar bundle rather than the
single-item NAR byte stream served by substitute servers.

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
- The dashboard server indexes catalog events internally, so `/api/catalog`
  works for automation even when no browser WebSocket is connected.

Dashboard events related to seeding:

| Event | Description |
|-------|-------------|
| `SeedAdded` | Emitted when a nar is added to the local store (startup seeding or post-download) |
| `BlockServed` | Emitted when blocks are served to a requesting peer (includes nar hash, peer, indices) |

## Dashboard Catalog View

The web dashboard includes a **catalog** panel showing packages discovered
during substitute queries. Each entry shows the store path name, nar size, and
whether P2P providers are available.

- `/api/catalog` returns all catalog entries seen by this node
- `CatalogEntry` events are emitted by the daemon when a `have` query
  processes a store path with narinfo metadata
- A dashboard-owned catalog listener records `CatalogEntry` events whether or
  not a browser is currently connected to `/ws`
- P2P availability is updated when DHT provider lookups succeed

Dashboard API endpoints:

- `/api/status` returns the local peer id, uptime, connected peer count, DHT
  entry count, observed build count, and seed count.
- `/api/peers` returns full peer ids and reputation counters.
- `/api/builds` returns observed builds with a `lookup_key` for detail links.
- `/api/build/{hash}` accepts either the registry lookup key or the nar hash.
- `/api/catalog` and `/api/seeds` return deterministic sorted snapshots.
