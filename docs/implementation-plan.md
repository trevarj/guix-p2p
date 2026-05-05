# Implementation Plan

## Estimated Effort: 6-10 weeks (libp2p path)

---

## Phase 1: Skeleton + Identity (Week 1)

### Goals
- Project scaffolding with all dependencies
- libp2p swarm initialization
- Ed25519 keypair generation and persistence
- CLI argument parsing

### Tasks

- [x] Create project directory and docs
- [x] `cargo init` with `Cargo.toml` listing all dependencies
- [x] `src/main.rs`: clap CLI with `--query` / `--substitute` modes
- [x] libp2p swarm setup: tokio runtime, QUIC transport, Noise auth
- [x] `src/behaviour.rs`: define GuixP2PBehaviour (kad + request-response + mdns + identify)
- [x] `src/config.rs`: configuration structs (bootstrap peers, block size, cache paths, timeouts)
- [x] `src/identity.rs`: Ed25519 keypair generation, save/load from disk, derive PeerId
- [x] Basic event loop that initializes the swarm and prints connected peers

### Deliverables
- [x] Compiling project with libp2p swarm that bootstraps against configured peers

---

## Phase 2: Daemon Protocol + DHT (Week 2-3)

### Goals
- Parse the daemon's stdin pipe protocol
- Wire Kademlia DHT lookups into "have" / "info" / "substitute" commands
- Reply correctly on fd 4

### Tasks

- [x] `src/daemon.rs`: stdin line reader
  - Parse `"have /gnu/store/..."` (one or more paths)
  - Parse `"info /gnu/store/..."` (one path)
  - Parse `"substitute /gnu/store/... /tmp/dest-path"` (one path + dest)
  - ReplyWriter with fd 4 write support via libc
  - Trace output formatting (download-started/progress/succeeded)
- [x] `src/narinfo.rs`: narinfo parser
  - Parse key:value pairs with signature boundary handling
  - URL/Compression/FileSize parsing from unsigned portion
  - `choose_best_url()` for compression selection
- [x] `src/fallback.rs`: HTTP substitute client
  - `fetch_narinfo()` tries each configured substitute URL
  - `download_nar()` downloads + decompresses + hash-verifies nars
  - Gzip decompression support via flate2
- [x] `src/dht.rs`: libp2p-kad wrapper
  - `bootstrap()` dials configured seed nodes
  - Async provider cache: `ProviderCache` with `handle_kad_event()` + `has_providers()`
  - Event-based cache population from Kademlia events
- [x] Wire "have" command:
  - For each path, extract 32-char hash part from store path
  - Send DHT query trigger via mpsc channel
  - Reply with paths that have cached providers
- [x] Wire "info" command:
  - Fetch narinfo from HTTP fallback servers
  - Reply with structured info (path, deriver, ref count, refs, download size, nar size)
- [x] Wire "substitute" command:
  - Fetch narinfo for metadata
  - HTTP fallback download (swarm is Phase 3)
  - Reply "success", "not-found", or "hash-mismatch" on fd 4
- [x] Concurrent architecture: swarm task + daemon stdin tasks in main.rs

### Deliverables
- [x] `guix-p2p-substitute --query` fully functional with DHT-backed "have" replies and HTTP-backed "info" metadata
- [x] `guix-p2p-substitute --substitute` functional with HTTP fallback download
- [x] Narinfo parsing with correct signed/unsigned boundary handling
- [x] Async DHT provider cache bridged to synchronous daemon protocol

---

## Phase 3: Swarm Downloader (Week 4-6)

### Goals
- Implement custom block exchange protocol over libp2p request-response
- Parallel block downloading from multiple peers
- Block and full-nar verification

### Tasks

- [x] `src/swarm/block.rs`:
  - Compute block count and individual block sizes from nar_size + block_size
  - SHA-256 hash of individual blocks
  - Bitfield type for tracking block availability
  - `split_nar(data, block_size) → Vec<Block>`
  - `join_blocks(blocks) → Vec<u8>`
  - `verify_nar(data, expected_hash) → bool`
- [x] `src/swarm/codec.rs`:
  - Define protocol message types (HANDSHAKE, REQUEST, BLOCKS)
  - Implement `RequestResponseCodec` for serializing/deserializing messages
  - Protocol name: `"/guix/substitute/0.1.0"`
- [x] `src/swarm/downloader.rs`:
  - Connection pool: up to 8 peers per nar download
  - Send handshake (nar_hash + block bitfield) to each peer
  - Rarest-first scheduler: track which peers have which blocks from handshake bitfields
  - Request pipeline: issue next REQUEST before current BLOCKS response arrives
  - Progress callback (for reporting to daemon)
  - Per-block SHA-256 verification on receipt
  - Final verification: SHA-256(block_0 || ... || block_N) == narinfo NarHash
- [x] Wire "substitute" command:
  - Call `get_providers(nar_hash)` for peer list
  - Initiate swarm downloader with discovered peers
  - On success: write nar to dest path, reply `"success sha256:... <nar_size>"` on fd 4
  - On timeout/stall: trigger HTTP fallback

### Deliverables
- Full nar download via swarm network from DHT-discovered peers

---

## Phase 4: HTTP Fallback + Narinfo Verification (Week 7)

### Goals
- Narinfo fetching from official substitute servers with signature verification
- Safety thresholds: minimum provider count, stall detection
- Narinfo caching with TTL
- Let guix-daemon chaining handle HTTP nar downloads (reply not-found on swarm failure)

### Tasks

- [x] `src/narinfo.rs`:
  - Parse narinfo format (key: value pairs)
  - Extract fields: StorePath, NarHash, NarSize, References, Deriver, URL, Compression, FileSize
  - Parse Signature field: version;hostname;base64-signature
  - Compute SHA-256 of signed portion (everything above Signature: line)
  - `load_acl_keys()` to parse `/etc/guix/acl` s-expression format
  - `verify_narinfo_signature()` to validate Ed25519 signatures against ACL public keys
  - `NarinfoCache` struct with 60s TTL, shared via `std::sync::Mutex`
- [x] `src/http_client.rs` (renamed from `fallback.rs`):
  - `fetch_narinfo(config, hash_part, cache) → Narinfo` with cache check + ACL verification
  - Tries each configured substitute URL, skips 404s
  - Signature verification errors return `BadSignature`
  - Removed HTTP nar download (redundant with guix-daemon's substituter chain)
- [x] Safety thresholds:
  - `config.min_providers` (default 3): fewer DHT results → skip swarm, reply not-found
  - `config.stall_timeout_secs` (default 30s): no new blocks within window → abort, reply not-found
  - Nar hash verification failed → reply not-found
  - Narinfo signature verification failed → return error, no fallback (security)
- [x] Graceful degradation:
  - Removed `download_nar()`, `handle_substitute_http()`, `choose_best_url()`
  - Removed `flate2` dependency (only used by dropped nar download)
  - Added `acl_path` and `stall_timeout_secs` fields to `Config`
  - Added `BadSignature` and `Other` error variants to `HttpClientError`
  - On swarm failure → reply "not-found" to daemon, let guix-daemon fall through

### Deliverables
- Narinfo fetch with Ed25519 signature verification and 60s TTL cache
- Swarm-only substitute delivery; guix-daemon chains to HTTP substituters on failure

---

## Phase 5: Hardening (Week 8-9)

### Goals
- Error resilience, connection management, reputation
- Background daemon mode for warm DHT routing table
- LAN auto-discovery, integration tests

### Tasks

- [x] Peer reputation:
  - Track completed/failed transfers per peer
  - Prefer peers with good history for future downloads
  - Time-decay scoring
  - Persist reputation to disk (JSON)
- [x] Connection management:
  - Timeout for peer connections (configurable, default 30s)
  - Retry with exponential backoff (max 3 retries)
  - Prune dead connections from peer pool (every 5 min)
  - Max total peers enforcement
- [x] Background daemon mode (`--daemon` flag):
  - Keep libp2p swarm alive between substituter invocations
  - Periodic DHT republishing tick (placeholder for local nar announcements)
  - Accept substitute requests while swarm is alive
- [x] Incoming block request serving:
  - Respond to Handshake requests with available block info
  - Error response for GetBlocks (local block storage not yet implemented)
- [x] mDNS auto-enable for LAN peer discovery (already working)
- [x] Bandwidth limiter (token-bucket, configurable upload/download rate caps)
- [x] Integration tests:
  - Daemon protocol wire format parsing
  - Reputation scoring and ban threshold
  - Connection manager retry/prune behaviour
  - Block utilities count and hash management
  - Narinfo parsing edge cases
- [x] NarinfoCache TTL eviction fix
- [x] Restructured as lib crate + binary for integration test support

### Deliverables
- Production-quality error handling, warm startup, LAN discovery

---

## Phase 6: Ship (Week 10)

### Goals
- Community bootstrap node deployment
- Performance benchmarking
- Guile wrapper patch
- Release

### Tasks

- [ ] Community bootstrap nodes:
  - Recruit 3+ volunteers to run long-lived libp2p bootstrap nodes
  - Document bootstrap node setup (systemd/shepherd service)
  - Update default bootstrap peer list in config
- [ ] Performance benchmarking:
  - HTTP only (baseline): download 10/100/500 MB nar
  - Swarm with 3/5/8 peers: compare throughput
  - Mixed (swarm + HTTP for cold start): measure improvement
- [ ] Guile wrapper: apply 4-line patch to `guix/scripts/substitute.scm`
- [ ] README with:
  - Quick start (install, set env var, confirm it works)
  - Bootstrap peer list and how to add your own node
  - Architecture overview
  - Configuration reference
- [ ] Guix channel with package definition
- [ ] Tag release v0.1.0

### Deliverables
- Usable P2P substitute client with community bootstrap infrastructure

---

## Out of Scope (Post-MVP / Future)

| Feature | Reason |
|---------|--------|
| NAT traversal / hole-punching | libp2p has built-in support (autonat/relay/dcutr); needs relay infrastructure |
| Bandwidth accounting / ratio enforcement | Requires persistent peer state; not needed for MVP utility |
| Incentive / token economics | Intentional non-goal |
| Web UI / dashboard | Separate project |
| IPFS integration | `(guix ipfs)` in Guix is unused dead code; could be wired later |
| Tor onion service DHT bootstrapping | Nice-to-have for censorship resistance |
| Store-level deduplication across peers | Hard; existing Guix dedup is local filesystem only |
