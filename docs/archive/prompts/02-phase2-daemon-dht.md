# Phase 2: Daemon Protocol + DHT

## Prerequisites
Phase 1 is complete: the project compiles, libp2p swarm initialises, identity/keypair works.

## Goal
Parse the guix-daemon's stdin pipe protocol ("have", "info", "substitute").
Wire Kademlia DHT provider lookups into query mode.
Fetch narinfo metadata from HTTP (fallback) for "info" queries.
Reply correctly on fd 4 with the format the daemon expects.

## Daemon Protocol Reference

The daemon spawns the substituter in one of two modes and communicates via stdin/stdout:

### Query Mode (`guix substitute --query`)
Reads lines from stdin, replies via fd 4 (not stdout):

**"have" query:**
```
stdin:  have /gnu/store/abc...-foo /gnu/store/def...-bar /gnu/store/ghi...-baz
fd 4:   /gnu/store/abc...-foo
fd 4:   \n                    (empty line = end of reply)
```
Only paths for which substitutes exist are returned.

**"info" query:**
```
stdin:  info /gnu/store/abc...-foo
fd 4:   /gnu/store/abc...-foo    (path)
fd 4:   /gnu/store/xyz...-foo.drv  (deriver, or empty line if none)
fd 4:   3                        (number of references)
fd 4:   /gnu/store/ref1
fd 4:   /gnu/store/ref2
fd 4:   /gnu/store/ref3
fd 4:   54321                     (download size in bytes)
fd 4:   12345                     (nar size in bytes)
fd 4:   (empty line = end)
```

### Substitute Mode (`guix substitute --substitute`)
```
stdin:  substitute /gnu/store/abc...-foo /tmp/guix-dest-xyz
fd 4:   success sha256:abc123... 12345
        OR
fd 4:   hash-mismatch sha256:expected... sha256:got... /gnu/store/abc...-foo
        OR
fd 4:   not-found /gnu/store/abc...-foo
```
Note: stdout (fd 1) carries build traces: `@ download-started ...`, `@ download-progress ...`, `@ download-succeeded ...`.
fd 4 carries the result line ONLY for substitute mode. In query mode, fd 4 carries structured data.

## Tasks

### 1. src/daemon.rs — Implement the protocol parser and reply writer

**Protocol parser:**
- `pub enum DaemonCommand { Have(Vec<String>), Info(String), Substitute { path: String, dest: String } }`
- `pub fn read_command() -> io::Result<DaemonCommand>`
  - Read a line from stdin (blocking)
  - Parse: if starts with "have ", collect space-separated paths
  - If starts with "info ", collect the single path
  - If starts with "substitute ", collect path and dest

**Reply writer:**
- `pub fn write_reply_fd4(line: &str)` — write a line + newline to fd 4
  - Open `/proc/self/fd/4` or use raw fd 4 via libc/nix
  - Actually: just use `std::os::unix::io::FromRawFd` to wrap fd 4 as a File, then BufWriter
  - Write line + `\n`, flush
- `pub fn write_reply_fd4_end()` — write just a newline (empty line = end marker)

### 2. src/daemon.rs — Implement run_query_mode()

```rust
pub async fn run_query_mode(
    swarm: &mut Swarm<GuixP2PBehaviour>,
    config: &Config,
) -> anyhow::Result<()> {
    loop {
        let cmd = read_command()?;
        match cmd {
            DaemonCommand::Have(paths) => handle_have(swarm, &paths).await?,
            DaemonCommand::Info(path) => handle_info(config, &path).await?,
            DaemonCommand::Substitute { .. } => {
                // Should not happen in query mode; ignore or log warning
            }
        }
    }
}
```

**handle_have:**
- For each path in `paths`:
  - Extract the 32-char hash part from the store path
    - Store path format: `/gnu/store/<32-char-hash>-<name>`
    - Find the hash by extracting chars 11..43 (after `/gnu/store/`, before `-`)
  - Call `dht::get_providers(swarm, &hash_part)` → returns Vec<PeerId> or empty
  - If providers exist → write the path to fd 4 (one per line)
- After all paths processed → write empty line to fd 4

**handle_info:**
- Fetch narinfo from HTTP for this path
  - Use `reqwest::get("{substitute_url}/{hash_part}.narinfo")` 
  - Try each substitute URL in config order
  - Parse narinfo (use `narinfo.rs` parser)
- If narinfo fetched and signature valid:
  - Write path to fd 4
  - Write deriver to fd 4 (or empty line)
  - Write reference count to fd 4
  - Write each reference to fd 4
  - Write download size to fd 4
  - Write nar size to fd 4
  - Write empty line to fd 4
- If narinfo not available → write path, then empty line (no data)

### 3. src/daemon.rs — Implement run_substitute_mode()

```rust
pub async fn run_substitute_mode(
    swarm: &mut Swarm<GuixP2PBehaviour>,
    config: &Config,
) -> anyhow::Result<()> {
    loop {
        let cmd = read_command()?;
        match cmd {
            DaemonCommand::Substitute { path, dest } => {
                handle_substitute(swarm, config, &path, &dest).await?;
            }
            _ => { /* ignore others in substitute mode */ }
        }
    }
}
```

**handle_substitute (stub for now, full implementation in Phase 3):**
- Extract hash part from path
- Attempt swarm download (stub → return false for now)
- If swarm fails → attempt HTTP fallback (`fallback::download_nar()`)
- On success → write `"success sha256:... <nar_size>"` to fd 4
- On failure → write `"not-found <path>"` to fd 4
- On hash mismatch → write `"hash-mismatch ..."` to fd 4

### 4. src/dht.rs — Implement Kademlia wrapper

```rust
/// Query the DHT for providers of a nar hash.
/// Returns empty Vec if no providers found (bootstrap may still be in progress).
pub fn get_providers(swarm: &mut Swarm<GuixP2PBehaviour>, nar_hash: &str) -> Vec<PeerId> {
    let hash = Multihash::wrap(Code::Sha2_256, &hex::decode(nar_hash).expect("valid hex"))
        .expect("valid multihash");
    swarm.behaviour_mut().kad.get_providers(hash);
    
    // Return empty for now — provider results come back asynchronously
    // via KademliaEvent::OutboundQueryProgressed in the event loop.
    // We store results in a local cache (HashMap<Key, Vec<PeerId>>) 
    // and query the cache synchronously in get_providers().
    // TODO Phase 5: implement async bridge
    Vec::new()
}

/// Bootstrap the DHT from configured seed peers.
pub async fn bootstrap(swarm: &mut Swarm<GuixP2PBehaviour>, peers: &[String]) -> anyhow::Result<()> {
    for addr in peers {
        match addr.parse::<Multiaddr>() {
            Ok(addr) => {
                tracing::info!("Bootstrapping from {}", addr);
                swarm.dial(addr)?;
            }
            Err(e) => {
                tracing::warn!("Invalid bootstrap address {}: {}", addr, e);
            }
        }
    }
    // After dialing, the kad protocol will issue FIND_NODE queries
    // as part of its automatic bootstrap process
    Ok(())
}
```

**Critical design note for async bridging:**
`kad.get_providers()` returns results asynchronously through the event loop.
The daemon protocol requires synchronous replies. Two options:
- **Option A (MVP)**: Maintain a `HashMap<Key, Vec<PeerId>>` cache. 
  Populate it from `KademliaEvent::OutboundQueryProgressed` events in main event loop.
  `get_providers()` queries the cache. First-time queries may miss.
  This is acceptable because the daemon retries "have" queries on subsequent builds.
- **Option B**: Split the event loop — one task for the swarm, another for daemon commands.
  Use a `tokio::sync::oneshot` channel to bridge async kad results to sync daemon reply.

Implement Option A for MVP simplicity.

### 5. src/main.rs — Wire modes

Update main.rs to:
- Parse CLI subcommand ("query" / "substitute")
- In query mode: call `daemon::run_query_mode(swarm, config)`
- In substitute mode: call `daemon::run_substitute_mode(swarm, config)`

### 6. src/swarm/mod.rs
Create empty module declarations for `block`, `codec`, `downloader` (stubs for Phase 3).

### 7. src/narinfo.rs — Stub parser
- `pub struct Narinfo { pub store_path: String, pub nar_hash: String, pub nar_size: u64, pub references: Vec<String>, pub deriver: Option<String>, pub urls: Vec<(String, String, u64)>, pub signature: Option<String> }`
- `pub fn parse_narinfo(data: &str) -> Result<Narinfo, ParseError>` — parse key: value lines
- `pub fn verify_signature(narinfo: &Narinfo, acl_keys: &[Vec<u8>]) -> bool` — stub for now

### 8. src/fallback.rs — Stub
- `pub async fn fetch_narinfo(config: &Config, hash_part: &str) -> Result<Narinfo>`
  - Try each substitute URL, return first successful
- `pub async fn download_nar(config: &Config, narinfo: &Narinfo, dest: &Path) -> Result<(String, u64)>`
  - reqwest GET the nar URL, decompress, write to dest, return (hash, size)

## Deliverables
- `"have"` command parses paths, extracts hash parts, queries local DHT cache, writes matching paths to fd 4
- `"info"` command fetches narinfo via HTTP, writes structured metadata to fd 4
- `"substitute"` command parses path+dest, attempts HTTP fallback, writes result to fd 4
- DHT bootstrap connects to configured seed peers and prints connection status
- Binary runs in both `--query` and `--substitute` modes

## Verification
- `cargo fmt` passes
- `cargo clippy --all-targets --all-features -- -D warnings` passes
- `cargo build` succeeds
- Can parse store paths and extract 32-char hash parts correctly
- fd 4 writing works (test with `echo "have /gnu/store/abc...-foo" | ./target/debug/guix-p2p-substitute query 4>&1`)
- HTTP narinfo fetch works against ci.guix.gnu.org
