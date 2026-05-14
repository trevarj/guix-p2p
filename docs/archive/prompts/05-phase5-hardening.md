# Phase 5: Hardening

## Prerequisites
Phase 4 is complete: swarm download works, HTTP fallback is reliable, narinfo verification works.

## Goal
Production-quality error handling, connection resilience, peer reputation,
background daemon mode for warm DHT routing table, bandwidth limiting,
LAN peer auto-discovery, and comprehensive integration tests.

## Tasks

### 1. Peer Reputation System

```rust
// src/reputation.rs

pub struct ReputationTracker {
    peers: HashMap<PeerId, PeerScore>,
}

pub struct PeerScore {
    completed: AtomicU32,
    failed: AtomicU32, 
    bytes_served: AtomicU64,
    total_response_time: AtomicU64, // cumulative microseconds
    response_count: AtomicU32,
    last_seen: RwLock<Option<Instant>>,
    bans: Vec<BanRecord>,
}

impl PeerScore {
    /// Score from 0.0 to 1.0. Time-decay weighted.
    pub fn score(&self) -> f64 {
        let completed = self.completed.load(Ordering::Relaxed) as f64;
        let failed = self.failed.load(Ordering::Relaxed) as f64;
        if completed + failed == 0.0 { return 0.5; } // neutral for new peers
        completed / (completed + failed + 1.0)
    }

    pub fn record_success(&self, bytes: u64, response_time: Duration);
    pub fn record_failure(&self, reason: FailureReason);
    pub fn record_ban(&self, duration: Duration, reason: &str);
    pub fn is_banned(&self) -> bool;
    pub fn average_response_time(&self) -> Duration;
}
```

Integration with downloader:
- Before selecting peers for a download, sort by `score()` descending
- After completed transfer: `reputation.record_success(peer_id, bytes, elapsed)`
- After failed transfer: `reputation.record_failure(peer_id, reason)`
- After hash mismatch or repeated timeouts: `reputation.record_ban(peer_id, 1h, "hash_mismatch")`
- Persist reputation to disk periodically (every 5 minutes, on shutdown)

### 2. Connection Resilience

```rust
// src/connection.rs

pub struct ConnectionManager {
    retry_config: RetryConfig,
    active_connections: HashMap<PeerId, ConnectionState>,
}

pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(16),
        }
    }
}
```

Connection handling:
- Dial with timeout (default 10s)
- On failure: exponential backoff (1s → 2s → 4s → give up at 3 retries)
- Connection health check: send periodic ping (every 30s if idle)
- Dead connection detection: if no response for 60s, drop and reconnect
- Prune stale connections every 5 minutes
- Max total peers: 50 (to avoid resource exhaustion)

### 3. Background Daemon Mode

```rust
// src/main.rs — add --daemon flag handling

/// When --daemon is passed, the binary runs as a long-lived p2p node:
/// - Maintains warm DHT routing table
/// - Periodically announces locally available nars
/// - Serves block requests from other peers
/// - Stays alive between daemon substitute invocations
pub async fn run_daemon_mode(config: &Config) -> anyhow::Result<()> {
    // Set up signal handlers for graceful shutdown
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        let _ = shutdown_tx.send(());
    });

    // Periodic tasks
    let mut announce_tick = tokio::time::interval(Duration::from_secs(3600)); // hourly
    let mut republish_tick = tokio::time::interval(Duration::from_secs(22 * 3600)); // 22h
    let mut persist_reputation_tick = tokio::time::interval(Duration::from_secs(300)); // 5min

    loop {
        tokio::select! {
            _ = shutdown_rx => break,
            _ = announce_tick.tick() => announce_local_nars(swarm, config).await?,
            _ = republish_tick.tick() => republish_dht_records(swarm).await?,
            _ = persist_reputation_tick.tick() => reputation.save_to_disk()?,
            event = swarm.select_next_some() => handle_swarm_event(event).await?,
        }
    }

    tracing::info!("Daemon shutting down gracefully");
    Ok(())
}
```

**Announce local nars:**
```rust
async fn announce_local_nars(swarm: &mut Swarm<...>, config: &Config) -> anyhow::Result<()> {
    // Walk /gnu/store/ (or configured store dir)
    // For each valid store item:
    //   Compute nar hash (or read from narinfo cache)
    //   Call dht::provide()
    // Limit to most recent 100 items to avoid excessive announces
}
```

### 4. Bandwidth Limiter

```rust
// src/bandwidth.rs

use tokio::sync::Semaphore;
use std::sync::Arc;

pub struct BandwidthLimiter {
    download_limit: Option<usize>, // bytes per second, None = unlimited
    upload_limit: Option<usize>,
    download_tokens: Arc<Semaphore>,
    upload_tokens: Arc<Semaphore>,
}

impl BandwidthLimiter {
    pub fn new(download_limit: Option<usize>, upload_limit: Option<usize>) -> Self {
        Self {
            download_limit,
            upload_limit,
            download_tokens: Arc::new(Semaphore::new(download_limit.unwrap_or(usize::MAX))),
            upload_tokens: Arc::new(Semaphore::new(upload_limit.unwrap_or(usize::MAX))),
        }
    }

    /// Acquire permission to transfer `bytes`. Blocks until tokens available.
    pub async fn acquire_download(&self, bytes: usize) {
        if let Some(limit) = self.download_limit {
            if bytes > limit { return; } // allow single large blocks to pass
            let permits = bytes.min(limit);
            // Refill tokens at configured rate using a background task
            self.download_tokens.acquire_many(permits as u32).await.unwrap().forget();
        }
    }
}
```

Integrate into:
- Swarm block sender (upload limiting)
- Swarm block receiver (download limiting)
- HTTP fallback downloader (download limiting)

### 5. mDNS LAN Auto-Discovery

libp2p-mdns is already configured in the behaviour. It just works:
- Peers on the same LAN automatically discover each other
- No configuration needed
- Verify this works by running two instances on the same network

Add logging for mDNS events:
```rust
SwarmEvent::Behaviour(GuixP2PEvent::Mdns(mdns::Event::Discovered(peers))) => {
    for (peer_id, addr) in peers {
        tracing::info!("Discovered LAN peer: {} at {}", peer_id, addr);
    }
}
```

### 6. Integration Tests

```rust
// tests/harness.rs — test harness for spinning up test networks

pub struct TestNetwork {
    nodes: Vec<TestNode>,
}

pub struct TestNode {
    peer_id: PeerId,
    swarm: Swarm<GuixP2PBehaviour>,
    addr: Multiaddr,
}

impl TestNetwork {
    /// Create N interconnected nodes on loopback.
    pub async fn new(count: usize) -> Self { ... }
    
    /// Have specific nodes provide a nar hash (for a generated test nar).
    pub async fn provide_nar(&mut self, node_indices: &[usize], nar_hash: &str) { ... }
    
    /// Download a nar from the network.
    pub async fn download_nar(&mut self, from: usize, nar_hash: &str) -> Vec<u8> { ... }
}
```

Test cases:

1. **Basic swarm download** — 3 nodes, one has nar, one downloads
2. **Multi-provider** — 5 nodes, 3 have the nar, verify load balancing
3. **Provider failure** — node disconnects mid-download, other nodes compensate
4. **Hash mismatch** — corrupt block detected, peer rejected, download succeeds from other peer
5. **Swarm fallback** — DHT has no providers, HTTP fallback succeeds
6. **Reputation scoring** — one peer serves fast, one slow; fast peer preferred
7. **Concurrent downloads** — two nar downloads simultaneously, bandwidth shared fairly
8. **Daemon protocol** — pipe-formatted input, verify fd 4 output format

### 7. Error Handling Audit

Review every function for proper error handling:
- No bare `.unwrap()` or `.expect()` in production code paths
- All errors are typed (enum variants) with useful context
- Timeout errors are distinguishable from logic errors
- Network errors trigger retries, not panics
- fd 4 is always written to eventually (daemon must not hang)

### 8. Logging Consistency

- All swarm events: `tracing::debug!`
- Provider lookups: `tracing::info!`
- Block transfers: `tracing::trace!` (verbose, per-block)
- Download progress: `tracing::info!`
- Errors: `tracing::error!` with full context
- Use structured fields where useful: `tracing::info!(nar_hash = %hash, peer_count = count, "starting swarm download")`

## Deliverables
- Peer reputation tracking with persistence
- Connection retries with exponential backoff
- Background daemon mode keeps DHT warm and serves peers
- Bandwidth limiting for fair resource sharing
- mDNS LAN discovery working automatically
- Comprehensive integration test suite
- Zero unwraps in production paths
- Consistent structured logging

## Verification
- `cargo fmt` passes
- `cargo clippy --all-targets --all-features -- -D warnings` passes
- `cargo test` passes (all integration tests green)
- Manual daemon mode test: run binary with --daemon, verify it stays alive and announces nars
- Stress test: download 100 nars via swarm, verify all succeed or fall back correctly
- Bandwidth limit test: set 1 MB/s limit, verify throughput is capped
