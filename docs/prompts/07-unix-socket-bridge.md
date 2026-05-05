# Phase 7: Unix Socket Bridge + Daemon Refactor

## Why

The current architecture has a fundamental disconnect:

- `guix-p2p --query` / `--substitute` processes do a **cold start** every time guix-daemon invokes them. Each invocation initialises a fresh libp2p swarm, creates empty caches, and builds a DHT routing table from scratch. The process then exits, losing all state.
- `guix-p2p --daemon` keeps a warm swarm alive but **nobody talks to it** because guix-daemon spawns separate `guix substitute --query` processes with their own stdin/fd4 pipes.

The fix: a Unix socket bridge. The daemon binds a local socket. Substitute invocations become thin relays that connect to the socket and forward bytes. Only the daemon runs the swarm.

## Architecture After This Phase

```
┌── shepherd service (persistent) ─────────────────────────────┐
│ guix-p2p --daemon --socket /run/user/1000/guix-p2p.sock     │
│   [warm libp2p swarm]  [DHT routing]  [connected peers]     │
│   [ProviderCache]      [BuildRegistry] [dashboard :3031]    │
│   [ReputationTracker]  [NarinfoCache]                       │
│   accepts Unix socket connections                           │
└──────────────────────────────────────────────────────────────┘
         ▲                         ▲
         │ Unix socket              │ Unix socket
    ┌────┴──────────────┐     ┌────┴──────────────┐
    │ guix-p2p --query  │     │ guix-p2p --sub    │
    │ --socket ...      │     │ --socket ...      │
    │ stdin→daemon      │     │ stdin→daemon      │
    │ daemon→fd4        │     │ daemon→fd4        │
    │ exits             │     │ exits             │
    └───────────────────┘     └───────────────────┘
```

Guix-daemon spawns `guix substitute --query` and `guix substitute --substitute`.
The PATH wrapper intercepts these and execs `guix-p2p --query --socket <path>` or
`guix-p2p --substitute --socket <path>`. These relay processes:

1. Connect to the daemon's Unix socket
2. Pipe stdin bytes into the socket
3. Read reply lines from the socket
4. Write each reply line to fd 4
5. Exit

The daemon processes each connection in sequence (guix-daemon serialises
substitute downloads). All shared state (swarm, caches, reputation) lives
in the daemon process.

## Implementation Tasks

Complete each task in order. After every task that writes code, run
`cargo build --workspace` and fix any compilation errors.

---

### Task 0: Rename binary

Rename the binary from `guix-p2p-substitute` to `guix-p2p`.

Files to change (find and replace only the **package name** / binary reference, not module paths):
- `Cargo.toml`: `name = "guix-p2p"`
- `e2e/Cargo.toml`: path dependency name `guix-p2p = { path = ".." }`
- `src/main.rs`: `#[command(name = "guix-p2p", version)]`
- `src/behaviour.rs`: `with_agent_version(format!("guix-p2p/{}", env!("CARGO_PKG_VERSION")))`
- `scripts/guix-wrapper.sh`: `exec guix-p2p "$@"` (line 27)
- All `.rs` files that have `guix_p2p_substitute::` in imports — rename to `guix_p2p::`
- `tests/integration.rs`: rename all `guix_p2p_substitute::` to `guix_p2p::`
- All `.md` files in `docs/` — replace `guix-p2p-substitute` with `guix-p2p`
- `README.md` — all references
- `AGENTS.md` — directory references
- `manifest.scm` — package name reference if present

**Verification:** `cargo build --workspace` compiles cleanly. All 60 tests still pass.

---

### Task 2: Add socket_path to Config

**File: `src/config.rs`**

Add one field to the `Config` struct (after the existing `tor_only` field):

```rust
pub socket_path: String,
```

In `Config::load()`, add a default:

```rust
socket_path: String::new(),
```

**Verification:** `cargo build` compiles. `cargo test -p guix-p2p` passes.

---

### Task 3: Add --socket CLI flag

**File: `src/main.rs`**

Add one CLI argument to the `Cli` struct (after the `tor_only` field):

```rust
/// Unix socket path for daemon to bind and relay to connect
#[arg(long, global = true)]
socket: Option<String>,
```

In `main()`, after the existing Tor config block:

```rust
if let Some(ref sock) = cli.socket {
    config.socket_path = sock.clone();
}
```

**Do NOT change the if/else routing yet.** That is Task 6.

**Verification:** `cargo build` compiles. `cargo run -- --help` shows `--socket` in the help.

---

### Task 4: Create relay module

**New file: `src/relay.rs`**

This module provides the function that `--query --socket X` and `--substitute --socket X` call. It connects to the daemon, forwards stdin, and writes replies to fd 4.

```rust
use std::io::BufRead;

use anyhow::Context;

/// Connect to the daemon's Unix socket, forward stdin bytes to it,
/// read reply lines and write them to fd 4.
pub async fn forward(socket_path: &str) -> anyhow::Result<()> {
    let stream = tokio::net::UnixStream::connect(socket_path).await
        .context("failed to connect to daemon socket")?;

    let (mut socket_read, mut socket_write) = stream.into_split();

    // Spawn background task to copy stdin into the socket
    let copy_task = tokio::spawn(async move {
        tokio::io::copy(&mut tokio::io::stdin(), &mut socket_write).await
    });

    // Read lines from the socket, write to fd 4
    use tokio::io::AsyncBufReadExt;
    let mut reader = tokio::io::BufReader::new(socket_read);
    let mut line = String::new();

    let fd: std::os::unix::io::RawFd = 4;

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break; // daemon closed
        }
        // Write the line to fd 4
        let buf = line.as_bytes();
        unsafe {
            let written = libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len());
            if written < 0 {
                return Err(anyhow::anyhow!("fd 4 write failed: {}", std::io::Error::last_os_error()));
            }
        }
    }

    // Wait for the copy task (discard result)
    let _ = copy_task.await;

    Ok(())
}
```

Add the import for `libc` at the top if not already present.

**Add to `src/lib.rs`:**

```rust
pub mod relay;
```

**Verification:** `cargo build` compiles. (The module won't be used until Task 6, so no runtime test yet.)

---

### Task 6: Route --query/--substitute with --socket to relay

**File: `src/main.rs`**

In `main()`, before the swarm initialization block (the lines starting with `let keypair = identity::load_or_generate_keypair...`), redirect the flow when both a mode flag and a socket are present.

Replace the section that handles `if cli.query` / `else if cli.substitute` / `else if cli.daemon`:

```rust
// If a socket path is provided with --query or --substitute,
// skip all swarm init and act as a relay.
if (cli.query || cli.substitute) && let Some(ref sock) = cli.socket {
    relay::forward(sock).await?;
    return Ok(());
}
```

Leave the rest of the if/else block unchanged — the existing cold-start code paths for `--query` and `--substitute` remain as fallbacks when no `--socket` is given.

**Note:** The `let Some(ref sock) = cli.socket` syntax above is a let chain. If your Rust version doesn't support `let` chains in if conditions, use a match instead:

```rust
match (cli.query || cli.substitute, &cli.socket) {
    (true, Some(sock)) => {
        relay::forward(sock).await?;
        return Ok(());
    }
    _ => {}
}
```

**Verification:** `cargo build` compiles. `cargo test --workspace` passes all tests.

---

### Task 7: Refactor daemon mode to accept Unix socket connections

**File: `src/daemon.rs`**

Current `run_daemon_mode` reads stdin in a loop, watching for `Substitute` commands. We replace the stdin reader with a Unix socket listener. Each connection gets the same daemon protocol processing.

Replace the current `run_daemon_mode` function body:

```rust
pub async fn run_daemon_mode(
    cache: &ProviderCache,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    mut notify_rx: tokio::sync::mpsc::UnboundedReceiver<SwarmNotification>,
    narinfo_cache: &Mutex<NarinfoCache>,
    config: &Config,
    reputation: &Arc<Mutex<ReputationTracker>>,
    conn_mgr: &Arc<Mutex<ConnectionManager>>,
    build_registry: &BuildRegistry,
    event_tx: &dashboard::EventBus,
    client: &reqwest::Client,
) -> anyhow::Result<()> {
    use std::io::{BufRead, BufReader, Write};
    use tokio::time::Duration;

    // --- dashboard (unchanged) ---
    if config.dashboard_enabled {
        let state = dashboard::DashboardState {
            provider_cache: cache.clone(),
            reputation: reputation.clone(),
            conn_mgr: conn_mgr.clone(),
            build_registry: build_registry.clone(),
            started: std::time::Instant::now(),
            peer_id: String::new(),
            event_bus: event_tx.clone(),
        };
        let port = config.dashboard_port;
        let bind = config.dashboard_bind.clone();
        let bind_clone = bind.clone();
        tokio::spawn(async move {
            dashboard::serve(state, port, &bind_clone).await;
        });
        tracing::info!("Dashboard enabled on http://{}:{}", bind, port);
    }

    // --- maintenance tick ---
    let republish_interval = Duration::from_secs(22 * 3600);
    let mut republish_tick = tokio::time::interval(republish_interval);

    // --- socket listener ---
    let sock_path = config.socket_path.clone();
    if sock_path.is_empty() {
        return Err(anyhow::anyhow!(
            "daemon mode requires --socket <path>"
        ));
    }

    // Remove stale socket file if present
    let _ = std::fs::remove_file(&sock_path);

    let listener = tokio::net::UnixListener::bind(&sock_path)
        .context("failed to bind Unix socket")?;

    tracing::info!("Daemon listening on {}", sock_path);

    loop {
        tokio::select! {
            _ = republish_tick.tick() => {
                tracing::debug!("Daemon republish tick (no local nars to announce)");
                conn_mgr.lock().unwrap().prune_dead();
                reputation.lock().unwrap().prune_stale(Duration::from_secs(30 * 24 * 3600));
            }
            accept = listener.accept() => {
                let (stream, _) = accept.context("socket accept failed")?;
                let (stream_read, mut stream_write) = stream.into_split();

                // Read daemon protocol lines from the socket
                let mut buf_reader = BufReader::new(stream_read);
                let mut line = String::new();

                loop {
                    line.clear();
                    match buf_reader.read_line(&mut line) {
                        Ok(0) => break, // EOF — connection closed
                        Ok(_) => {},
                        Err(e) => {
                            tracing::warn!("socket read error: {}", e);
                            break;
                        }
                    }

                    let cmd = match parse_command_line(line.trim()) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!("parse error: {}", e);
                            let _ = writeln!(stream_write, "error {}", e);
                            continue;
                        }
                    };

                    // Build a ReplyWriter that writes to the socket stream
                    let mut reply = SocketReplyWriter::new(stream_write);

                    match cmd {
                        DaemonCommand::Have(paths) => {
                            // handle_have needs query_tx + ProviderCache.
                            // We don't have query_tx in the daemon context.
                            // Instead, check the ProviderCache directly and reply.
                            for path in &paths {
                                if let Ok(hash_part) = extract_hash_part(path) {
                                    if crate::dht::has_providers(cache, &hash_part).await {
                                        let _ = reply.write_line(path);
                                    }
                                }
                            }
                            let _ = reply.write_end();
                        },
                        DaemonCommand::Info(path) => {
                            handle_info(config, narinfo_cache, &mut reply, &path, client).await;
                        },
                        DaemonCommand::Substitute { path, dest } => {
                            try_swarm_substitute(
                                config, cache, cmd_tx,
                                &mut reply, &path, &dest,
                                &mut notify_rx, narinfo_cache,
                                reputation, build_registry, event_tx,
                                client,
                            ).await;
                        },
                    }

                    stream_write = reply.into_inner();
                }
            }
        }
    }
}

/// A ReplyWriter that sends lines over a Unix stream (std::io::Write)
/// instead of fd 4.
struct SocketReplyWriter<W: std::io::Write> {
    writer: W,
}

impl<W: std::io::Write> SocketReplyWriter<W> {
    fn new(w: W) -> Self { SocketReplyWriter { writer: w } }

    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        write!(self.writer, "{}\n", line)
    }

    fn write_end(&mut self) -> std::io::Result<()> {
        write!(self.writer, "\n")
    }

    fn into_inner(self) -> W { self.writer }
}
```

Now update `handle_info` to accept the new writer type. Make it generic over the writer:

```rust
async fn handle_info<W: std::io::Write>(
    config: &Config,
    cache: &Mutex<NarinfoCache>,
    reply: &mut SocketReplyWriter<W>,
    path: &str,
    client: &reqwest::Client,
) {
    // ... same body, just uses SocketReplyWriter instead of ReplyWriter
}
```

But `try_swarm_substitute` also uses `ReplyWriter`. The simplest approach: keep the existing `ReplyWriter` for fd4, and create a separate `SocketReplyWriter` for the daemon. Both have `.write_line(&str)` and `.write_end()`.

Actually better: extract a trait. But for minimal diff, just have two writer types with the same method signatures. Since `try_swarm_substitute` is called from both `run_substitute_mode` (fd4) and `run_daemon_mode` (socket), we need to abstract the writer.

Simplest approach without a trait: pass a **callback** or use an **enum**. Let's keep it simple — the `try_swarm_substitute` function in the daemon already takes a `&mut ReplyWriter`. We'll change `ReplyWriter` to be generic by making it an enum:

```rust
pub enum ReplyWriter {
    Fd4,
    Socket(Box<dyn std::io::Write + Send>),
}

impl ReplyWriter {
    pub fn new_fd4() -> Self { ReplyWriter::Fd4 }

    pub fn new_socket(w: impl std::io::Write + Send + 'static) -> Self {
        ReplyWriter::Socket(Box::new(w))
    }

    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        match self {
            ReplyWriter::Fd4 => { /* existing libc::write code */ }
            ReplyWriter::Socket(ref mut w) => write!(w, "{}\n", line),
        }
    }

    pub fn write_end(&mut self) -> io::Result<()> {
        match self {
            ReplyWriter::Fd4 => { /* existing blank line code */ }
            ReplyWriter::Socket(ref mut w) => write!(w, "\n"),
        }
    }
}
```

In `run_daemon_mode`, create a `ReplyWriter::new_socket(stream_write)` from the accepted connection stream.

**Verification:** `cargo build --workspace` compiles cleanly. `cargo test --workspace` passes all existing tests.

---

### Task 8: Update wrapper script

**File: `scripts/guix-wrapper.sh`**

Change the exec line (27) to include `--socket`:

```sh
exec guix-p2p "$@" --socket "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/guix-p2p.sock"
```

This passes `--socket /run/user/UID/guix-p2p.sock` to every `guix-p2p --query` and `guix-p2p --substitute` invocation.

**Verification:** The wrapper tests in `scripts/test-wrapper.sh` still pass after updating the mock binary name.

---

### Task 9: Update documentation

**File: `docs/architecture.md`**

Update the data flow diagram to show the socket bridge replacing the cold-start path. Update the activation section to describe daemon + relay model.

**File: `docs/implementation-plan.md`**

Add a Phase 7 entry after Phase 5 (before Phase 6):

```
### Phase 7: Unix Socket Bridge + Daemon Refactor

- [ ] Rename binary guix-p2p-substitute → guix-p2p
- [ ] Add socket_path to Config
- [ ] Add --socket CLI flag
- [ ] Create relay module (src/relay.rs)
- [ ] Route --query/--substitute with --socket to relay
- [ ] Refactor daemon mode to accept Unix socket connections
- [ ] Update wrapper script with socket path
- [ ] Update architectural docs
```

**File: `README.md`**

Update all references from `guix-p2p-substitute` to `guix-p2p`.

**Files: `docs/e2e-demo-guide.md`**

Update references to the new binary name.

---

## Global Rules

After each task that modifies source code:
1. Run `cargo build --workspace` and fix all compilation errors
2. Do NOT proceed to the next task until the current task builds clean
3. Do NOT run `cargo fmt`, `cargo clippy`, or `cargo test` until the final task
4. Never create new files outside the specified paths
5. Never modify files not listed in the current task
6. Never commit changes
7. Never run cargo fmt or cargo clippy — these will be run at the end
8. If you encounter a Rust feature you're unsure about, prefer a simpler approach that compiles

## Verification (Final)

After all tasks are complete:
1. `cargo build --workspace` must compile
2. `cargo test --workspace` — all existing tests must still pass
3. `cargo fmt`
4. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
5. The binary accepts `guix-p2p --daemon --socket /tmp/p2p-test.sock` without error
6. The binary accepts `guix-p2p --query --socket /tmp/p2p-test.sock` and exits cleanly (connecting to a running daemon)
