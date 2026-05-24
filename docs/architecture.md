# Architecture: guix-p2p

## Overview

`guix-p2p` is a standalone Rust binary that speaks the guix-daemon's
existing substituter pipe protocol, but sources binary substitutes (nars) from a
libp2p-powered Kademlia DHT + custom block-swarm network instead of (or
alongside) HTTP.

Zero changes to the guix-daemon. A Guix command extension shadows the internal
`guix substitute` command when the daemon environment includes the package's
extension directory in `GUIX_EXTENSIONS_PATH`. The extension delegates
substitute protocol traffic directly to the daemon Unix socket when the
`guix-p2p` socket is available.

## Architecture Overview

The binary runs in three operational modes plus diagnostics:
- **Daemon** (`--daemon`): Persistent process with warm libp2p swarm, listening
  on a Unix domain socket for relay connections
- **Relay** (`--query --socket PATH` or `--substitute --socket PATH`): Legacy
  Rust client that forwards stdin/fd4 through the Unix socket to the daemon.
  The Guix extension now uses an in-process Scheme socket client instead.
- **Direct** (`--query` or `--substitute` without `--socket`): Development and
  fallback path that initializes a fresh swarm for one substituter request.
- **Doctor** (`--doctor`): Local readiness checks for tester rollout. It loads
  config and identity, then reports bootstrap peer, shareable address, cache,
  socket, ACL, and substitute URL readiness without starting the swarm.
- **Init** (`--init`): Non-destructive first-run helper that writes a starter
  `$XDG_CONFIG_HOME/guix-p2p/config.toml` when no config exists.

## Data Flow

```
guix-daemon
  │ spawns "guix substitute --query" or "--substitute"
  │ stdin: "have /gnu/store/...", "info /gnu/store/...", "substitute /gnu/store/... /tmp/dest"
  │ fd 4:  reads "success sha256:... 12345" or "not-found" or "hash-mismatch ..."
  ▼
guix-p2p substitute extension from GUIX_EXTENSIONS_PATH
  │ if extension detects substitute protocol mode and socket exists → connect to daemon socket
  ▼
guix-p2p (Rust, libp2p)
  │
  ├─► Daemon Mode (--daemon)
  │     Unix socket listener at $XDG_CACHE_HOME/guix-p2p/guix-p2p.sock
  │     Accepts relay connections, processes query/substitute requests
  │     Warm libp2p swarm, seeds to peers
  │
  ├─► Relay Mode (--query --socket PATH / --substitute --socket PATH)
  │     Legacy/debug client that connects to daemon's Unix socket
  │     Sends mode header + forwards stdin to daemon
  │     Writes daemon replies to fd 4
  │     Avoids cold-start libp2p init but still starts a Rust process
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
  │     get_providers(nar_hash) → deduplicated burst of PeerIds
  │     start_providing(nar_hash) → announce availability
  │     Bootstrap from community-maintained seed nodes
  │
  ├─► Swarm Downloader (libp2p request-response streams)
  │     Handshake: nar_hash + advertised block indices
  │     Request: nar_hash + up to 8 block indices per batch
  │     Dynamic block assignment to the least-loaded peer advertising each block
  │     Per-block SHA-256 verification
  │     Final nar-SHA-256 verification against narinfo
  │     Peer reputation scoring (time-decay, ban threshold)
  │     Connection management (retry/backoff, dead peer pruning)
  │
  ├─► Bandwidth Limiter (leaky-bucket, configurable caps)
  │
  ├─► Diagnostics
  │    `--doctor` and dashboard `/api/status` connectivity summary
  │    Flags missing bootstrap peers, missing shareable addresses, private
  │    external addresses, stopped daemon socket, missing ACL, and substitute
  │    URL setup
  │
  ├─► Provider Probe
  │    `--test-provider-lookup HASH` dials bootstrap peers, queries Kad
  │    providers, and handshakes provider plus connected/bootstrap fallback
  │    candidates for the requested NAR hash
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
| Daemon integration | Unix socket relay + Guix substitute extension | Zero daemon C++ changes; relay gives <1ms startup |
| Narinfos | HTTP fetch from official substitute URLs, optional local metadata file for offline harnesses | Tiny (<500 bytes); existing trust chain unchanged for HTTP, while local metadata is reserved for explicit offline tests |
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
struct ReputationTracker { .. }     // src/reputation.rs: transport reliability scoring
struct ConnectionManager { .. }     // src/connection.rs: dial state, retry/backoff, pruning
struct BandwidthLimiter { .. }     // src/bandwidth.rs: shared leaky-bucket rate limiting
```

Reputation is deliberately transport-only. It ranks peers for block download
selection and stale-provider avoidance, but it does not authenticate narinfo,
attestations, or build outputs.

## Daemon Protocol Layer

The daemon protocol follows guix-daemon's substituter pipe protocol exactly.

### Socket relay protocol

The Unix socket between daemon and relay uses channel prefix framing:
- `fd4:<line>\n` — structured reply data (have paths, info metadata, success/not-found)
- `out:<line>\n` — trace output (`@ download-started`, `@ download-succeeded`)
- `nar:<base64-chunk>\n` / `nar-end\n` — verified NAR bytes for the current
  substitute request. The daemon streams these chunks directly to the socket
  instead of buffering a full base64-encoded NAR reply.

The extension socket client demuxes these: `fd4:` lines are written to fd 4,
`out:` lines to stdout (fd 1), and `nar:` chunks are decoded to the destination
from the `substitute <store-path> <dest>` command. Guix's substituter protocol
expects that destination to be a NAR file; `guix-daemon` restores it into the
store after the substituter reports `success` on fd 4.
Destination writes happen inside the substitute process spawned by
`guix-daemon`, not in the long-lived user daemon. This keeps the warm swarm
architecture while matching guix-daemon's permission model.
If the daemon socket closes during substitute mode before a terminal `fd4:`
reply (`success`, `not-found`, or `hash-mismatch`), the relay exits with an
error instead of silently reporting success.

### Query protocol

- **have**: daemon writes `have <path1> <path2> ...\n`. Reply: each available
  path on a separate line, terminated by blank line.
- **info**: daemon writes `info <path1> ...\n`. Reply per path: store_path,
  deriver, ref_count, refs, download_size, nar_size, then blank line.
  Narinfo derivers and references are returned as full `/gnu/store/...` paths.
  `have` only advertises paths with usable narinfo. In `p2p-only`, `info`
  replies are served from the narinfo cache only. This avoids turning unrelated
  Guix metadata queries into remote substitute-server timeouts; availability is
  enforced by `have` and the final `substitute` request.

`--local-narinfo PATH` or `local_narinfo_path = "PATH"` loads a JSON metadata
file into the narinfo cache at daemon startup. When this is set, substitute
metadata lookups are limited to that local cache instead of falling through to
remote substitute servers. The file has a top-level `narinfos` array with
`store_path`, `nar_hash`, `nar_size`, `references`, optional `deriver`, and
optional `download_size` fields. This is intended for offline/local benchmark
harnesses that pre-seed store paths and already know the corresponding NAR
hashes. VM benchmarks use remote substitute-server narinfo lookups for public
package, profile, and system-build p2p modes. System-build benchmarks are the
exception for the generated top-level system output: that local-only output is
used to compute the closure, but the timed benchmark fetches the public-narinfo
closure substitutes directly through the normal P2P/HTTP policy.
If local metadata has `nar_size = 0`, p2p downloads derive the block count from
the provider handshake and still verify the final NAR hash.

### Substitute protocol

- **substitute**: daemon writes `substitute <store-path> <dest>\n`. Reply:
  `success sha256:<hash> <size>`, or `hash-mismatch sha256 <expected> <actual>`,
  or `not-found`.

Before the download, `@ download-started <path> <url> <size>` is written to
the trace channel. After success, `@ download-succeeded <path> <url> <size>`
is written. P2P traces use a `p2p://<hash-part>` URL; HTTP traces use the
selected substitute URL. These match the guix-daemon build trace protocol.

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
     → kad.get_providers(nar_hash) → collect/deduplicate matching PeerIds
     → include path only if peers found
   "info" returns verified narinfo metadata without DHT-gating; availability is
   enforced by "have" and the final "substitute" request.

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

Provider lookup results are merged into the in-memory provider cache instead of
replacing earlier peers. A later sparse DHT response should not erase providers
observed by a previous `have` query.

## Swarm Block Protocol

Block size: 256 KiB (configurable)
Block hash = SHA-256(block_data)
Block count = ceil(nar_size / 256KiB)

```
Client → Peer: Handshake { nar_hash: Vec<u8> }
Peer → Client: HandshakeReply { blocks_available: Vec<u32>, block_count: u32, block_size: u32 }
Client → Peer: GetBlocks { nar_hash: Vec<u8>, indices: Vec<u32> }
Peer → Client: Blocks { data: Vec<BlockData> }
```

### Verification
- Per-block: SHA-256(block_data) must match known hash
- Final: SHA-256(block_0 || ... || block_N) must equal nar-SHA-256 from narinfo

### Block Selection
- Track availability per peer via handshake replies.
- Retry handshakes during the handshake window.
- Filter provider candidates through peer reputation and connection backoff
  before handshakes so stale DHT records are deprioritized after failures. The
  selection report keeps reputation filtering and connection-backoff filtering
  visible as separate reasons.
- If cached providers are filtered below `min_providers`, wait for fresh DHT
  provider notifications, then retry selection against the original providers
  plus any fresh providers before failing the P2P attempt.
- Require at least `min_providers` successful handshakes before downloading.
- Successful handshakes reset the peer's connection-backoff state, so repeated
  successful sequential downloads from the same provider do not self-throttle.
- Assign pending blocks to the least-loaded peer that advertises the block.
- Requeue stalled or invalid in-flight blocks for another provider.

### Parallelism
- Max 8 concurrent peer connections per nar download
- Keep up to `max_in_flight_blocks_per_peer` block requests active per peer.

## Anti-Spam

Peer announcements via `kad.start_providing()` include implicit data
availability. Before trusting a provider, validate:

1. Connect to peer, request random blocks via block_exchange
2. Verify SHA-256 for each received block
3. If verification fails, blacklist peer for this nar hash

Handshake timeouts and outbound request failures update peer reputation.
Repeated failures put the peer under connection-manager backoff before later
provider lists are tried. Expired records are still handled by libp2p-kad's
TTL-based record management.

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

Default: `http-first`. Sparse early-test networks should use HTTP first for
normal Guix work, then opt into `p2p-first` when deliberately validating peer
transfer behavior.

For `p2p-first`, small NARs up to 1 MiB use a short provider-discovery budget
before HTTP fallback. This avoids spending the full P2P request timeout on tiny
packages where HTTP can usually finish faster than peer discovery on a sparse
network. Larger NARs and `p2p-only` retain the normal `request_timeout_secs`
budget.

The policy also affects the `have` query:
- `http-first` and `p2p-first`: Always respond with the path (we can serve via HTTP fallback).
- `p2p-only`: Only respond if DHT providers exist for the nar hash.

### HTTP Nar Download

When the policy allows HTTP fallback, nars are downloaded from the substitute
server URLs in the narinfo. Decompression supports zstd, gzip, lzip, and
uncompressed NARs. The preference order is: zstd > gzip > lzip > none.
Unsupported compression entries are skipped when a supported URL is available;
otherwise the download fails before hash verification.
Relative narinfo URLs are tried against each configured substitute base URL in
order. Absolute narinfo URLs are fetched directly. If a preferred compression
URL fails, lower-preference URLs from the same narinfo are tried before HTTP
fallback gives up.
HTTP response chunks pass through `BandwidthLimiter` when
`max_download_rate_kbps` is set.
The same chunk stream emits `@ download-progress` traces with the HTTP source
URL, response length when known, and compressed bytes transferred.
Each HTTP candidate is decompressed and checked against the narinfo `NarHash`.
Hash mismatches fall through to the next HTTP candidate when one is available;
if every candidate fails integrity, the substituter still reports
`hash-mismatch`. If every candidate fails before integrity can be checked, the
HTTP client reports that all candidates failed and includes the last underlying
error.

### Safety thresholds:
- DHT returns < `min_providers` peers → skip swarm, reply not-found. The
  provider threshold defaults to `3` and can be set with `--min-providers` or
  `min_providers` in config.
- Swarm downloads use a size-scaled overall deadline with `request_timeout_secs`
  as the floor. Large nars are allowed to keep running while block progress is
  flowing.
- Swarm download stalls (no new blocks for `stall_timeout_secs` (30s)) → abort, reply not-found
- Nar hash verification failed → reply not-found
- Narinfo signature verification failed for one substitute URL → try the next
  configured substitute URL; if none verify, report an error

Narinfo flow:
1. Check NarinfoCache (60s TTL, shared via `std::sync::Mutex`)
2. For each configured substitute URL, `reqwest` GET
   `<substitute_url>/<hash-part>.narinfo`
3. Verify each candidate's Guix SPKI signature with libgcrypt against
   `/etc/guix/acl` public keys
4. Decode `NarHash: sha256:<nix-base32>` to raw SHA-256 bytes for DHT/swarm
   keys
5. Cache and return the first verified Narinfo

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
| `lzma-rust2` | Lzip decompression for HTTP nar downloads |
| `leaky-bucket` | Shared async upload/download rate limiting |
| `if-addrs` | Local interface address detection for peer-address pruning |
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
.guix-channel                 # Guix channel metadata
guix.scm                      # Compatibility package entrypoint for local builds
channel/guix-p2p/
├── packages.scm              # Channel package module: (guix-p2p packages)
└── services.scm              # Channel service module: (guix-p2p services)
guix/
└── extensions/substitute.scm # Guix command extension installed by the package
src/
├── lib.rs                   # Crate root (public API for integration tests)
├── main.rs                  # CLI, config overlay, daemon/relay mode dispatch
├── behaviour.rs             # libp2p NetworkBehaviour (kad + block_exchange + mdns + identify)
├── channel.rs               # SwarmCommand / SwarmNotification enums (broadcast channel types)
├── config.rs                # Config struct (block_size, timeouts, ACL path, socket_path, etc.)
├── connection.rs            # ConnectionManager (retry/backoff, dead peer pruning, max peers)
├── daemon.rs                # Query/substitute pipeline, daemon mode, socket listener
├── daemon/
│   └── protocol.rs          # Daemon command parser, fd 4/socket reply writer, trace formatting
├── runtime.rs               # libp2p swarm task, command handling, block request serving
├── relay.rs                 # Unix socket relay client (stdin → socket → fd 4)
├── dht.rs                   # Kad wrapper, handle_kad_event → notifications, get_providers, bootstrap
├── reputation.rs            # ReputationTracker (time-decay scoring, ban threshold, JSON persistence)
├── bandwidth.rs             # BandwidthLimiter (leaky-bucket, configurable caps)
├── dashboard.rs             # Dashboard HTTP/WebSocket routing and state
├── dashboard/
│   ├── catalog.rs           # Catalog state and event upsert logic
│   ├── geo.rs               # Multiaddr IP extraction and country/flag helpers
│   ├── packages.rs          # Guix profile package discovery and parsing
│   └── seed_config.rs       # seed_paths TOML persistence/removal
├── http_client.rs           # Narinfo fetch (HTTP only), signature verification, cache
├── nar_hash.rs              # NarHash hex/Nix-base32 decoding helpers
├── narinfo.rs               # Narinfo parser, ACL loader, Ed25519 verifier, NarinfoCache (with TTL eviction)
├── nar_store.rs             # NarStore: local nar cache, block serving, raw single-item nar seeding
├── store_path.rs            # Guix store path parsing and normalization helpers
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
flag selects the legacy Rust relay mode for `--query` and `--substitute`.

### Daemon + Extension Architecture

The recommended deployment uses a persistent daemon and the Scheme substitute
extension as the socket client:

1. **Daemon** (`guix-p2p --daemon`): Listens on a Unix domain socket
   (`$XDG_CACHE_HOME/guix-p2p/guix-p2p.sock` by default). Keeps the libp2p
   swarm warm, serves block requests, processes queries and substitutes from
   relay connections.

2. **Extension socket client**: For `--query` and `--substitute`, connects to
   the daemon's Unix socket, sends a mode header (`mode: query\n` or
   `mode: substitute\n`), forwards stdin lines, writes substitute replies to fd
   4, writes trace output to stdout, and restores `nar:` payload chunks to the
   destination path.

Each socket connection sends a mode header and then streams daemon protocol
commands. The daemon processes each connection independently, subscribing to the
swarm's broadcast notification channel for that connection. The Rust
`--query --socket` / `--substitute --socket` relay remains available for
debugging and older wrapper deployments, but it is no longer the normal Guix
extension path.

### Guix Substitute Extension

The package installs `(guix extensions substitute)` under
`share/guix/extensions/substitute.scm`. Adding the package to a system profile
makes this available as
`/run/current-system/profile/share/guix/extensions/substitute.scm`, but Guix
does not scan that directory unless it is in `GUIX_EXTENSIONS_PATH`. The
`guix-daemon` service helper prepends that directory to the daemon's existing
`GUIX_EXTENSIONS_PATH`, so Guix resolves this extension before its built-in
`guix substitute` implementation:

```
guix-daemon invokes "guix substitute --query"
  → extension intercepts "--query"
  → if socket exists: connect to $GUIX_P2P_SOCKET and relay each query line/reply
  → else: delegate to built-in guix substitute --query

guix-daemon invokes "guix substitute --substitute"
  → extension intercepts "--substitute"
  → if socket exists: connect to $GUIX_P2P_SOCKET, receive daemon nar output, and restore it into Guix's destination path
  → else: delegate to built-in guix substitute --substitute

other substitute invocations
  → delegate to built-in guix substitute
```

Integration contract:

- `GUIX_EXTENSIONS_PATH` lets Guix find `(guix extensions substitute)`.
- `GUIX_P2P_SOCKET` tells the extension where the warm relay daemon is
  listening.
- Query mode is interactive: the extension must write the reply for each
  `have`/`info` command before waiting for stdin EOF, because `guix-daemon`
  keeps the query process open.
- Substitute mode receives base64-framed raw NAR bytes from the daemon, stores
  them in a temporary file, restores them into Guix's requested destination
  with `(guix serialization) restore-file`, and then forwards the terminal
  `success` or failure reply on fd 4. Like query mode, substitute mode is
  interactive: the extension handles one `substitute` command and terminal
  reply before waiting for another stdin line.
- `GUIX_P2P_BIN` is only used by older relay/wrapper setups; the Scheme
  extension does not exec it.

No `GUIX` wrapper is required for the recommended path.

The Rust `guix-p2p-wrapper` binary remains installed for compatibility with
older setups that set the daemon's `GUIX` environment variable.

### Peer Address Hygiene

Addresses learned from LAN mDNS may include private LAN addresses and are only
used for local discovery. Addresses learned from identify are treated as
non-LAN advertisements and must be publicly dialable DNS/IP multiaddrs before
they are added to the DHT, connection manager, or persistent peer store. This
prevents a public bootstrap node from repeatedly dialing a client's loopback,
home-LAN, or VPN interface addresses.

### Provider Announcements

Seeded NARs are announced with Kademlia `start_providing` at daemon startup and
again whenever a peer connection is established. The connection-time announce
matters for bootstrap nodes that start with no bootstrap peers of their own:
their first startup announce may not have any Kad peers to publish to yet, but
they still need later clients to discover cached seeds.

### Shepherd Service (Daemon Mode)

Guix System deployments should use `guix-p2p-service-type`. Normal daemon
network settings are first-class service fields, so system daemon bootstrap
configuration lives in the operating-system service definition rather than a
user's TOML file:

```scheme
(service guix-p2p-service-type
         (guix-p2p-configuration
          (dashboard? #t)
          (external-addresses '())
          (policy "http-first")))
```

The service defaults to the project bootstrap node and `http-first` policy. Set
`(bootstrap-peers '())` to run without default bootstrap peers, or set
`(policy "p2p-first")` for transfer-validation sessions.

For standalone bootstrap-node operations, use the Shepherd-first guide in
[`bootstrap-node.md`](bootstrap-node.md). The equivalent low-level service shape
is:

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
substitute_policy = "http-first"
bootstrap_peers = "/ip4/1.2.3.4/udp/6881/quic-v1/p2p/QmPeer1,/ip4/5.6.7.8/udp/6881/quic-v1/p2p/QmPeer2"
peer_store_enabled = true
peer_store_max_entries = 100
external_addresses = "/dns4/node.example.org/udp/6881/quic-v1"
substitute_urls = "https://bordeaux.guix.gnu.org,https://ci.guix.gnu.org"
min_providers = 3
request_timeout_secs = 30
stall_timeout_secs = 30
block_size = 262144
max_peers_per_download = 8
max_in_flight_blocks_per_peer = 4
max_upload_rate_kbps = 0
max_download_rate_kbps = 0
max_total_peers = 50
acl_path = "/etc/guix/acl"
seed_paths = ["/gnu/store/abc-foo", "/gnu/store/def-bar"]
```

### E2E Harnesses

The `guix-p2p-e2e container-smoke` harness has its own CLI surface for local
proofs. It supports `--dashboard-bind` for VM port forwarding and `--hold` to
keep validated smoke-test dashboards running until Ctrl-C. These flags do not
change production daemon configuration. Container smoke exercises the copied
Scheme substitute extension directly for `--query` and `--substitute`; full
`guix-daemon` integration remains covered by VM e2e because the isolated
container daemon can leave a spawned query substituter idle without sending a
query line.

The strict store-isolation proof uses `guix-p2p-e2e vm`, which builds one
qcow2 Guix System base image and auto-creates named node disks from it. Any
node can act as bootstrap, seeder, or fetcher for a scenario. Its state
defaults to `target/guix-p2p-e2e`.

VM nodes are assigned independent host-side port ranges for SSH (2221+),
dashboard (3031+), and P2P (6881+). Each port type is allocated separately so
that an occupied port in one range does not cause other ranges to skip. Inside
each VM, the dashboard and P2P daemon always listen on fixed guest ports (3031
and 6881 respectively), and QEMU host forwarding maps the unique host-side port
to the fixed guest port.

### Future: Upstream Hook

If upstream accepts a daemon-owned substituter hook, the extension becomes
unnecessary. The cleaner upstream shape is `guix-daemon
--substituter-program=FILE`, where the custom program receives `--query` or
`--substitute` directly and speaks the existing substituter pipe protocol.

## Seeding Strategy

Nars are seeded from a local cache directory after successful downloads or
explicit `--seed` paths. The `NarStore` (`src/nar_store.rs`) manages storage,
serving, and seed provenance labels.

### Approach: Hybrid nar cache seeding

- **Post-download seeding**: By default, successful P2P downloads are saved to
  `<cache_dir>/nar/<sha256hex>.nar`, tagged as `downloaded`, and announced in
  the DHT via `start_providing`. HTTP fallback downloads are not auto-seeded
  unless `auto_seed_downloads = "all"` is configured. Set
  `auto_seed_downloads = "off"` to disable download auto-seeding entirely.
  Seed metadata is persisted beside the NAR as
  `<cache_dir>/nar/<sha256hex>.json`.
- **Explicit seeding**: The `--seed` flag accepts comma-separated store paths.
  Each is hashed via `guix hash -S nar -f hex`, serialized as a raw
  single-item NAR with Guix's `(guix serialization) write-file`, and stored in
  the nar cache with source `manual`. All seeded nars are announced in the DHT
  on startup.
- **Startup scan**: On startup, `NarStore::new()` scans `<cache_dir>/nar/*.nar`
  and indexes each file by its filename stem (the hex sha256). Files whose
  bytes do not hash to the filename stem are skipped. Matching sidecar metadata
  restores the source tag, store path, and creation time. Files whose
  provenance is not known are tagged as `cache`. Cache files without sidecar
  metadata can later be annotated when trusted narinfo/catalog metadata for
  that NAR hash is observed. All indexed nars are annotated with any matching
  local narinfo metadata and announced in the DHT.
- **Serving**: Incoming block requests are served from the nar store. The
  `NarStore::handle_request()` method dispatches to handshake replies (with
  block hashes) or block data reads. Outbound responses pass through
  `BandwidthLimiter` when `max_upload_rate_kbps` is set. The old
  `serve_block_request()` stub has been replaced.

### NarStore wire-up

- `NarStore` is wrapped in `Arc<Mutex<NarStore>>` and shared between:
  - The swarm task (serves incoming block requests)
  - The daemon substitute mode (saves nars after successful downloads)
  - The daemon socket listener (same as substitute mode, for relay connections)
- After saving a nar, a `SwarmCommand::StartProviding { hash }` is sent to the
  swarm task to announce the new nar in the DHT.
- In daemon mode, a background health monitor periodically queries provider
  counts for local seeds, emits provider-count dashboard events, and
  re-announces local seeds when known providers fall below `min_providers`.
- Downloads query Kad first, then handshake the returned providers. If Kad has
  too few usable providers, the downloader handshakes connected peers and
  configured bootstrap peer IDs before falling back to HTTP. A fallback peer is
  only used after it returns a normal block handshake for the requested NAR.
  When that fallback path is attempted, the Guix trace stream includes a
  `p2p+connected-fallback://<hash>` download-started marker before any HTTP
  fallback marker.

### `--seed` CLI flag

```
guix-p2p --daemon --seed /gnu/store/...-foo,/gnu/store/...-bar
```

Each path is fed to `NarStore::seed_store_path()`, which runs `guix hash` and
Guix's raw NAR serializer. System services resolve these helpers from
`/run/current-system/profile/bin` when available, then fall back to `PATH`.
The serializer subprocess also receives `GUILE_LOAD_PATH`,
`GUILE_LOAD_COMPILED_PATH`, and `GUILE_EXTENSIONS_PATH` for the system profile
so it can load `(guix serialization)` under Shepherd's sparse service
environment. `GUIX_P2P_GUILE` can point at a specific Guile executable when a
test harness or service wrapper needs the same interpreter used by the active
Guix command. It intentionally does not use `guix archive --export`, because
that command writes a signed nar bundle rather than the single-item NAR byte
stream served by substitute servers.

## Dashboard Seeding View

The web dashboard (`--dashboard`) includes a **seeds** panel that shows all
locally-seeded nars in real time:

- **Seed list**: Each seeded nar is displayed with source tag (`manual`,
  `auto`, or `cache`), package/store path when known, hash, size, and block
  count. Cache-only rows show that metadata is pending instead of pretending
  the hash is a package name. Clicking a row opens a detail overlay with the
  reason the NAR is being seeded.
- **Seed serialization**: explicit store-path seeds are serialized with Guix's
  NAR writer. When running inside `guix shell`, the seeder prefers
  `GUIX_ENVIRONMENT` commands and Guile module paths; system services fall back
  to `/run/current-system/profile`.
- **Real-time events**: `BlockServed` events stream via WebSocket showing
  which blocks are being uploaded to which peers. Served rows flash green
  momentarily.
- **Reload-safe event history**: dashboard events are retained in a daemon
  memory ring buffer and exposed through `/api/events`, so browser reloads
  restore the latest operational history. The web dashboard renders this
  history newest-first, keeps the event filters pinned above the list, and
  scrolls the event list from the top so recent failures remain visible. This
  history is intentionally not persisted across daemon restarts.
- **Transfer evidence**: accepted `BlockReceived` events and non-empty
  `BlockServed` events are aggregated by peer, so the transfer path shows
  which peers contributed downloaded blocks and which requesters received
  served blocks. The transfer detail overlay also shows phase timings for
  narinfo, provider lookup, handshakes, HTTP/P2P download, verification,
  import, and total substitute time when those events were observed.
- **DHT evidence**: provider lookup and provider announce events are retained
  in the event stream. Successful downloads include a source label:
  `p2p-dht`, `p2p-connected-fallback`, or `http-fallback`.
- **Seed count** in the header bar updates as nars are seeded or auto-saved
  after downloads.
- The `/api/seeds` endpoint returns the full list of seeded nars with size,
  block count, block size, store path when known, seed source, and cache
  creation time.
- The `/api/packages` endpoint lists installed packages from
  `/run/current-system/profile`, `/run/current-system/kernel`, and
  `$HOME/.guix-home/profile` using
  `guix package --list-installed --profile=<profile>`. The daemon prefers
  `/run/current-system/profile/bin/guix` so system services do not depend on a
  shell `PATH`, then falls back to `guix`. Each entry includes the source
  profile, package name, version, output, store path, and whether that store
  path is already seeded by the local NAR store.
- The dashboard package panel consumes `/api/packages`, supports fuzzy search
  across package name, version, and store path, and shows source, output, store
  path, and seed state. Its row-level seed control calls `POST /api/seeds`.
- The dashboard layout is organized for seeder management. The header includes
  a compact readiness control that always shows node health, connectivity
  role, and latest transfer source. Version/commit, uptime, peer/DHT summary,
  share/bootstrap details, and tester gates open from that control into the
  existing side drawer, so expanded details never overlap the transfer
  workspace. The header keeps connectivity to one truncating summary line to
  avoid overlap with long multiaddrs or diagnostic strings.
- The primary desktop workspace is a 12-column operations grid: Transfer Path
  gets the widest first-row area, Active Seeds sits beside it for seeding
  inventory, Packages is the secondary add-seed list, Peers and Discovery share
  the middle row, and Events gets a full-width bottom lane for readable failure
  and transfer history.
- Catalog and Builds are merged into one Discovery panel. It combines
  substitute lookup rows with narinfo/build metadata, showing store path,
  size, P2P/HTTP availability, and provider count when known. Rows with build
  metadata still open the existing signed narinfo/build detail overlay.
- Events remain a full-width bottom log for recent failures, connectivity
  trace, and transfer proof breadcrumbs. The event row has a larger minimum
  height and wraps messages instead of squeezing them into a thin strip.
- Detailed counts live in the panels that own them. The header intentionally
  avoids duplicating package, seed, peer, catalog, DHT, and uptime counters.
- The tester checklist appears only in the node details drawer and summarizes
  the setup gates that usually block a real first-run P2P substitute test:
  Guix integration, bootstrap peers, connected peers, observed transfer
  activity, and whether the node is directly shareable or client-only.
- The dashboard header uses an inline `guix-p2p` SVG wordmark so the embedded
  dashboard can render the logo without a separate static asset route.
- The main dashboard grid scrolls as a whole when zoom or viewport height makes
  the fixed panels taller than the available space, so the bottom Events panel
  remains reachable instead of being clipped.
- Each main dashboard panel exposes one faded `?` affordance in the top-right
  corner. Clicking it opens a concise overview for that panel, including the
  important controls and row elements inside it.
- Dashboard HTML keeps actions in data attributes and uses delegated click and
  keyboard handlers for dynamic rows, filters, help, overlays, and raw narinfo
  toggles. This keeps generated package/seed/discovery rows inspectable and
  avoids inline JavaScript handlers.
- Dashboard panels are semantic sections in visual/tab order: transfer proof,
  active seeds, packages, peers, discovery, and event log. The event stream is
  exposed as a polite log region, and grid cells that contain peer IDs,
  multiaddrs, store paths, or diagnostic strings explicitly allow truncation
  instead of overlapping adjacent controls.
- Dense panels render compact column headers for scanability: Active Seeds
  labels seed/cache/action, Packages labels package/state/action, Peers labels
  geo/peer/score/state, Discovery labels store item/size/path/providers, and
  Events labels time/type/message.
- Dashboard colors are selected from a named theme dropdown. Terminal yellow,
  green, and amber preserve the console look; Tokyo Night provides a more
  conventional high-contrast dark palette. Scrollbars follow the active theme,
  and active seed rows reserve separate columns for package path, size/block
  metadata, and stop controls to avoid overlap.
- Keyboard focus is visible on interactive row cards, detail affordances, and
  controls. Motion-heavy row flash and drawer animations respect
  `prefers-reduced-motion`. Transfer, seed, package, and discovery rows expose
  concise accessible labels so keyboard and assistive-technology users get the
  same operational context as pointer users.
  A dashboard unit test pins these embedded-HTML affordances so the refined
  layout, labels, and copy cannot silently regress.
- Active seed rows display the package/store item name when store-path metadata
  is available, with the full store path and NAR hash available in details.
  Hash-only rows include provenance text such as startup cache, manual cache
  entry, or P2P download plus `metadata pending`, so users can understand why a
  NAR is listed before narinfo/catalog metadata has identified the package.
  The seed detail drawer groups identity and cache/provenance fields and
  exposes the same stop action as the row, so a tester can inspect a seed and
  remove it without returning to the list.
  Each active seed row can stop seeding directly by NAR hash, which works even
  when older cached seeds do not have store-path metadata. Package rows remain
  seed-only; removal is intentionally centralized in Active Seeds.
- The transfer path panel is driven by dashboard events and tracks the latest
  observed package/NAR through package observation, narinfo trust, provider
  discovery, block movement, verification/import, and local re-seeding. The
  inline view stays compact with package, current proof stage, stage detail,
  size, block totals, peer counts, byte totals, and a six-stage progress strip.
  Clicking the panel opens the detail overlay with per-stage values, per-peer
  block counts, download byte counts, and the most recent block indices
  accepted from or served to each peer.
- `POST /api/seeds` accepts `{ "store_path": "/gnu/store/..." }` only when the
  dashboard bind address is loopback. It validates the path, seeds and caches
  the NAR immediately, sends `StartProviding`, emits `SeedAdded`, and persists
  the path to the user config `seed_paths` while preserving existing TOML where
  possible.
- `DELETE /api/seeds/{hash}` stops serving a local NAR, removes its cache file,
  removes the matching store path from persisted `seed_paths`, and emits
  `SeedRemoved`. Already-published Kademlia provider records are not actively
  withdrawn in this implementation; they expire according to libp2p/Kademlia
  provider-record behavior, while the local node immediately stops serving the
  removed NAR.
- The dashboard server indexes catalog events internally, so `/api/catalog`
  works for automation even when no browser WebSocket is connected.
- The dashboard server indexes transfer events internally, so `/api/transfers`
  returns aggregate download and upload evidence, phase timings, plus the last
  successful download source, even when no browser WebSocket is connected.
- The dashboard server indexes the latest 500 events internally, so
  `/api/events` restores recent history after browser reloads. The live
  WebSocket remains the source for newly-arriving events. The browser view
  compacts repeated near-identical events, especially repeated dial failures,
  so the event stream stays readable during connectivity problems.
- The peer panel is backed by connection-manager snapshots plus reputation
  records. Connected peers are shown even before they have reputation history,
  disconnected reputation-only peers remain visible, and peer rows use backend
  address, country, last-active data, and bootstrap/source labels instead of
  browser-only guesses.
- The embedded dashboard also understands the `demo` flag returned by the e2e
  demo server and marks the header status as demo data when present.

Dashboard events related to seeding:

| Event | Description |
|-------|-------------|
| `SeedAdded` | Emitted when a nar is added to the local store (startup seeding or post-download) |
| `BlockReceived` | Emitted when requested blocks are accepted from a provider (includes nar hash, peer, indices, bytes) |
| `BlockServed` | Emitted when blocks are served to a requesting peer (includes nar hash, peer, indices) |

## Dashboard Discovery View

The web dashboard includes a **discovery** panel showing packages discovered
during substitute queries plus matching narinfo/build metadata when available.
Each entry shows the store path name, nar size, whether P2P providers are
available, and the provider count from build metadata when known.

- `/api/catalog` returns all catalog entries seen by this node
- `/api/builds` returns observed narinfo/build records that the browser merges
  into the discovery panel
- `CatalogEntry` events are emitted by the daemon when a `have` query
  processes a store path with narinfo metadata
- A dashboard-owned catalog listener records `CatalogEntry` events whether or
  not a browser is currently connected to `/ws`
- P2P availability is updated when DHT provider lookups succeed

Dashboard API endpoints:

- `/api/status` returns the daemon version string, local peer id, listen
  address, configured external addresses, configured bootstrap peers, shareable
  multiaddrs, uptime, currently connected peer count, DHT entry count, observed
  build count, and seed count.
- `/api/share-info` returns the same bootstrap bundle as `guix-p2p
  --share-info --json`: PeerId, shareable multiaddrs, optional dashboard URL,
  diagnostic summary flags, and a paste-ready bootstrap config snippet.
- `/api/diagnostics` returns the daemon's doctor-style report with
  connectivity, readiness checks, and error/warning booleans.
- `/api/peers` returns full peer ids, connection state, known addresses,
  address count, last-active age, and reputation counters.
- `/api/builds` returns observed builds with a `lookup_key` for detail links.
- `/api/build/{hash}` accepts either the registry lookup key or the nar hash.
- `/api/transfers` returns aggregate transfer evidence by nar hash, including
  downloaded block counts, downloaded bytes, served block counts, last
  successful download source, phase timings in milliseconds, and peer-level
  contribution summaries.
- `/api/events` returns the latest 500 dashboard events with a monotonic
  in-memory id, timestamp, and serialized event payload. Connection events
  include outbound dial attempts, dial failures, inbound attempts, inbound
  failures, successful connects, and disconnect reasons when libp2p provides
  one.
- `/api/packages` returns installed system and Guix Home packages with seed
  state.
- `POST /api/seeds` seeds an existing local store path and persists it for
  future daemon starts.
- `DELETE /api/seeds/{hash}` stops serving a local seed and removes it from
  future daemon starts.
- `/api/catalog` and `/api/seeds` return deterministic sorted snapshots.
