# Phase 1: Skeleton + Identity

## Goal
Initialize the Rust project with all dependencies, set up the libp2p swarm,
implement Ed25519 keypair management, and wire up CLI argument parsing.
Nothing connects to the daemon yet — this is pure scaffolding.

## Tasks

### 1. cargo init
```
cargo init --name guix-p2p-substitute
```
Create the project in the current directory.

### 2. Cargo.toml
Add these dependencies (use latest versions from crates.io):
```
[dependencies]
libp2p = { version = "*", features = ["full", "kad", "quic", "tcp", "dns", "request-response", "mdns", "identify", "macros", "autonat", "relay", "dcutr"] }
tokio = { version = "1", features = ["full"] }
ed25519-dalek = "2"
sha2 = "0.10"
reqwest = { version = "0.12", features = ["rustls-tls", "stream"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
bitvec = "1"
anyhow = "1"
base64 = "0.22"
```

### 3. src/main.rs
- Use clap derive to define CLI:
  - Subcommands: `query` and `substitute`
  - `--bootstrap-peers <MULTIADDRS>` (optional, overrides defaults)
  - `--listen-addr <MULTIADDR>` (optional, default /ip4/0.0.0.0/udp/6881/quic-v1)
  - `--cache-dir <PATH>` (optional, for narinfo cache and key storage)
  - `--daemon` (optional flag, reserved for Phase 5)
- Initialize tracing subscriber with env filter (`RUST_LOG`)
- Load or generate identity (ed25519 keypair)
- Build libp2p swarm with QUIC + TCP transports, Noise authenticated encryption, yamux multiplexing
- Add all behaviours defined in `behaviour.rs`
- Start the swarm and enter event loop
- On CTRL+C or SIGTERM, gracefully shutdown

### 4. src/config.rs
```rust
pub struct Config {
    pub bootstrap_peers: Vec<String>,
    pub listen_addr: String,
    pub cache_dir: PathBuf,
    pub block_size: usize,        // default 262144 (256 KiB)
    pub request_timeout_secs: u64, // default 30
    pub max_peers_per_download: usize, // default 8
}
impl Config {
    pub fn load() -> Self { /* env vars + CLI + defaults */ }
}
```
Default bootstrap peers: use placeholder multiaddrs. Include a comment `// TODO: replace with real community bootstrap nodes before release`.

### 5. src/identity.rs
- `pub fn load_or_generate_keypair(cache_dir: &Path) -> Keypair`
  - Check `{cache_dir}/keypair` file
  - If exists: deserialize from bytes
  - If not: generate new ed25519 keypair via libp2p, save to disk
- `pub fn keypair_to_libp2p(keypair: &ed25519_dalek::SigningKey) -> libp2p::identity::Keypair`
  - Convert ed25519-dalek key to libp2p Keypair (needed for PeerId + swarm auth)
- `pub fn peer_id_from_keypair(kp: &libp2p::identity::Keypair) -> PeerId`

### 6. src/behaviour.rs
Define the aggregate NetworkBehaviour:
```rust
#[derive(NetworkBehaviour)]
pub struct GuixP2PBehaviour {
    pub kad: Kademlia<MemoryStore>,
    pub block_exchange: request_response::cbor::Behaviour<SwarmRequest, SwarmResponse>,
    pub mdns: tokio::mdns::Behaviour,  // or async-mdns
    pub identify: identify::Behaviour,
}
```
Define placeholder `SwarmRequest` and `SwarmResponse` enums (empty for now, filled in Phase 3):
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SwarmRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SwarmResponse {}
```
Implement `From` for each inner event into a unified `GuixP2PEvent`.

### 7. src/dht.rs (stub)
- `pub fn bootstrap(swarm: &mut Swarm<GuixP2PBehaviour>, peers: &[String])`
  - Dial each bootstrap peer
  - After bootstrapping, kick off iterative FIND_NODE toward own PeerId

### 8. src/daemon.rs (stub)
- `pub fn run_query_mode() -> anyhow::Result<()> { todo!() }`
- `pub fn run_substitute_mode() -> anyhow::Result<()> { todo!() }`

### 9. src/swarm/mod.rs
Create directory with mod.rs that declares submodules (empty for now).

## Deliverables
- `cargo build` succeeds
- `cargo run -- query` runs, initializes swarm, prints "Swarm listening on ...", and awaits events
- Keypair is persisted to `{cache_dir}/keypair` on first run
- Keypair is loaded from cache on subsequent runs (same PeerId across restarts)

## Verification
- `cargo fmt` passes
- `cargo clippy --all-targets --all-features -- -D warnings` passes
- `cargo build` succeeds
- The binary starts and the swarm initialises without panicking
