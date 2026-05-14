# Phase 6: Ship

## Prerequisites
Phase 5 is complete: hardened, tested, background daemon mode works.

## Goal
Prepare for public release. Set up community bootstrap infrastructure,
benchmark performance, write the Guile wrapper patch, create the Guix
channel package, and tag v0.1.0.

## Tasks

### 1. Community Bootstrap Infrastructure

Document how to run a bootstrap node:

**Bootstrap Node Setup (document file: docs/bootstrap-node.md):**
```md
# Running a Guix P2P Bootstrap Node

## Requirements
- Public IP address or port-forwarded NAT
- UDP port 6881 reachable (QUIC)
- Persistent storage for keypair and reputation

## Setup

1. Install guix-p2p-substitute
2. Create a systemd/Shepherd service:
   ```
   guix-p2p-substitute --daemon \
     --listen-addr /ip4/0.0.0.0/udp/6881/quic-v1 \
     --cache-dir /var/lib/guix-p2p \
     --bootstrap-peers <other_bootstrap_peers>
   ```
3. Share your multiaddr with the community to be added to the default list

## Monitoring
- Log output to file or journal
- Key metrics: connected peer count, k-bucket occupancy, bandwidth usage
```

**Default bootstrap list update:**
```rust
// src/config.rs — update placeholder with real addresses

const DEFAULT_BOOTSTRAP_PEERS: &[&str] = &[
    // TODO: replace with real community bootstrap nodes before v0.1.0
    // "/ip4/xxx.xxx.xxx.xxx/udp/6881/quic-v1/p2p/12D3KooW...",
    // "/dnsaddr/p2p.guix.example.org/udp/6881/quic-v1/p2p/12D3KooW...",
];

// When we have real nodes, add them here.
// Users should maintain at least 3 bootstrap nodes.
```

### 2. Performance Benchmarking

Create benchmarks in `benches/download.rs`:
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

// Benchmark with mock/test network (no real network dependency)

fn bench_block_hash(c: &mut Criterion) {
    let data = vec![0u8; 262144]; // 256 KiB
    c.bench_function("sha256_block", |b| {
        b.iter(|| sha2::Sha256::digest(black_box(&data)));
    });
}

fn bench_block_split_join(c: &mut Criterion) {
    let nar_data = vec![0x42u8; 100 * 1024 * 1024]; // 100 MB simulated nar
    let block_size = 262144;
    c.bench_function("split_and_join_100mb", |b| {
        b.iter(|| {
            let blocks = swarm::block::split_into_blocks(black_box(&nar_data), block_size);
            let joined = swarm::block::join_blocks(&blocks);
            assert_eq!(joined, nar_data);
        });
    });
}

criterion_group!(benches, bench_block_hash, bench_block_split_join);
criterion_main!(benches);
```

**Performance comparison doc (`docs/performance.md`):**

Measure and document:
- HTTP only: download 10 MB, 100 MB, 500 MB nars from ci.guix.gnu.org
- Swarm with 3 peers (LAN): same nars
- Swarm with 8 peers (LAN): same nars
- Cold start overhead: DHT lookup + handshake latency
- Memory usage during download (peak RSS)
- CPU usage during decompression + hash verification

### 3. Guile Wrapper Patch

Create the patch file: `guix/0001-guix-substitute-p2p-wrapper.patch`

```diff
diff --git a/guix/scripts/substitute.scm b/guix/scripts/substitute.scm
index ...
--- a/guix/scripts/substitute.scm
+++ b/guix/scripts/substitute.scm
@@ ... @@
   #:use-module (guix narinfo)
   #:use-module (guix pki)
   ...
-  #:export (guix-substitute))
+  #:export (guix-substitute
+            %p2p-substitute-program))
 
 ...
 
+(define %p2p-substitute-program
+  ;; The P2P substituter binary to exec when GUIX_USE_P2P is set.
+  (make-parameter "guix-p2p-substitute"))
+
+(define (maybe-exec-p2p-substituter)
+  "If GUIX_USE_P2P is set to 'yes', exec the P2P substituter binary."
+  (when (and=> (getenv "GUIX_USE_P2P")
+               (cut string-ci=? <> "yes"))
+    (dup2 (fileno (current-output-port)) 4)
+    (apply execlp (%p2p-substitute-program)
+           (%p2p-substitute-program)
+           (cdr (command-line)))))
+
 ...
 
 (define (guix-substitute . args)
+  (maybe-exec-p2p-substituter)
   (with-error-handling
     (match args
       (("--query")
```

### 4. Guix Channel Package

Create `guix.scm` in project root:
```scheme
;;; guix.scm — Guix package definition for guix-p2p-substitute
;;;
;;; Use as: guix build -f guix.scm
;;; Or add to a Guix channel for: guix install guix-p2p-substitute

(use-modules
 (guix packages)
 (guix build-system cargo)
 (guix licenses)
 (gnu packages crates-io)
 (gnu packages crates-crypto)
 (gnu packages crates-web)
 (gnu packages crates-graphics))  ; for bitvec etc.

(define-public guix-p2p-substitute
  (package
    (name "guix-p2p-substitute")
    (version "0.1.0")
    (source ...)  ; local or git source
    (build-system cargo-build-system)
    ;; NOTE: This is aspirational. The crates-io module may not exist yet
    ;; in Guix. For initial release, users install via cargo.
    ;; Full Guix packaging requires importing all crate dependencies.
    (home-page "https://example.org/guix-p2p")
    (synopsis "Peer-to-peer substitute distribution for GNU Guix")
    (description
     "guix-p2p-substitute is a binary substituter for GNU Guix that
distributes store items (nars) over a peer-to-peer network. It speaks
the guix-daemon substituter protocol and discovers peers via a
libp2p-powered Kademlia DHT.")
    (license gpl3+)))
```

Since Guix's `crates-io.scm` does not exist yet and fully packaging all Rust
dependencies for Guix is a massive side quest, for v0.1.0:
- Provide `cargo install` instructions in README
- Provide the patch file for manual application
- Defer full Guix packaging to post-v0.1.0

### 5. README Finalization

Update `README.md`:
- Installation (cargo install from git)
- Guix daemon configuration (set GUIX_USE_P2P=yes)
- Quick verification (guix build hello with p2p)
- Configuration reference (bootstrap peers, ports, bandwidth limits)
- Bootstrap node list (if nodes are available)
- Build from source instructions
- Link to all docs

### 6. CHANGELOG.md

```
# Changelog

## 0.1.0 (unreleased)

### Added
- Kademlia DHT for peer discovery via nar hashes
- Custom block-swarm protocol for parallel nar downloads
- HTTP fallback to official Guix substitute servers
- Narinfo parsing and Ed25519 signature verification
- Peer reputation and connection resilience
- Background daemon mode with local nar serving
- mDNS LAN peer auto-discovery
- Guile wrapper patch for guix substitute integration
```

### 7. Version Tag

```bash
git tag -a v0.1.0 -m "Initial release: P2P substitute distribution for GNU Guix"
```

### 8. Community Announcement Draft

Document in `docs/announcement.md`:
- What this project is
- How it works (architecture overview)
- How to install and try it
- Call for bootstrap node volunteers
- Link to the repo, docs, and the Guile wrapper patch

## Deliverables
- Bootstrap node documentation and setup guide
- Hardcoded default bootstrap list (placeholder if no real nodes yet)
- Performance benchmarks and comparison doc
- Guile wrapper patch file (ready to apply to Guix source)
- Guix package definition (aspirational, cargo install for now)
- Updated README with complete instructions
- CHANGELOG
- Git tag v0.1.0

## Verification
- README instructions work end-to-end for a new user
- Guile patch applies cleanly and doesn't break non-P2P path
- Performance doc has real numbers (not estimates)
- All tests pass in release mode: `cargo test --release`
- Binary runs with `--help` showing all options
- Binary runs with `--version` showing "guix-p2p-substitute 0.1.0"
