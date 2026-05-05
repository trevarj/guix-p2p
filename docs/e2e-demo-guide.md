# End-to-End Demo Guide

## Overview

The `e2e` crate provides an interactive multi-node demo that spins up two
libp2p QUIC nodes on your local machine, connects them, performs a block
handshake and nar download, and serves a live web dashboard.

One node (the **seeder**) seeds a simulated nar archive. The other node (the
**downloader**) bootstraps from the seeder, requests blocks via the
`/guix/substitute/0.1.0` protocol, and streams every event to the dashboard
over WebSocket.

## Quick Start

From the repository root:

```sh
# Build and run the demo
cargo run -p guix-p2p-e2e --example demo
```

Then open **http://127.0.0.1:3031** in a browser.

You will see a hacker-terminal dashboard showing:

- **Header bar** — node peer ID, connected peer count, DHT entries, uptime timer
- **Peers panel** — the seeder appears when the connection is established (flag
  emoji, score bar, transfer stats)
- **Builds panel** — the seeded nar hash with its provider count; click it to
  see the full detail panel with store path, size, references, deriver, and
  the raw signed narinfo text
- **Events panel** — a scrolling log of every event in real time:
  peer connected, handshake reply received, blocks arriving, download
  completion

## What Happens

```
1. Seeder starts on 127.0.0.1:<auto-port>/quic-v1
2. Downloader starts on 127.0.0.1:<auto-port>/quic-v1
3. Downloader dials seeder → ConnectionEstablished
4. Downloader sends HANDSHAKE { nar_hash }
5. Seeder replies HANDSHAKE_REPLY { blocks_available, block_hashes }
6. Downloader sends REQUEST { indices: 0..N }
7. Seeder replies BLOCKS { data: [(0, bytes), (1, bytes), ...] }
8. Each block triggers a BlockReceived event in the dashboard event log
```

All steps are visible live in the event log.

## Expected Output

```
╔══════════════════════════════════════════╗
║    guix-p2p-substitute  2-node demo     ║
╠══════════════════════════════════════════╣
║  nar hash: 6a2c...                       ║
║  nar size: 500000  blocks:   8           ║
╚══════════════════════════════════════════╝

seeder  pid=12D3KooW...  addr=/ip4/127.0.0.1/udp/12345/quic-v1/p2p/...
dash    pid=12D3KooX...  addr=/ip4/127.0.0.1/udp/12346/quic-v1/p2p/...
dashboard on http://127.0.0.1:3031
connected to 12D3KooW...
→ handshake request
← block handshake reply from 12D3KooW...: 8 blocks available
→ requesting 8 blocks
← received 8 blocks (488.3 KB) from 12D3KooW...
open http://127.0.0.1:3031 ← live events
press Ctrl-C to stop
```

Press **Ctrl-C** to shut down all nodes.

## Running With More Visibility

Increase log output to see every swarm event:

```sh
RUST_LOG=guix_p2p_substitute=trace,guix_p2p_e2e=trace \
  cargo run -p guix-p2p-e2e --example demo
```

Reduce to just block transfer events:

```sh
RUST_LOG=guix_p2p_e2e=info cargo run -p guix-p2p-e2e --example demo
```

## Troubleshooting

**Dashboard shows "no peers yet" or "no builds observed"**

The dashboard populates as events stream in via WebSocket. Wait a few seconds
— the connection and handshake take 2-5 seconds on the first run.

**Port 3031 already in use**

```sh
# Edit e2e/examples/demo.rs and change DASHBOARD_PORT, or kill the process
lsof -ti:3031 | xargs kill
```

**Nodes don't connect**

QUIC requires an available UDP port. If your loopback interface has firewall
rules blocking UDP, the demo won't work.

## Architecture

```
┌─────────────┐       ┌─────────────────────┐
│   Seeder    │       │     Downloader      │
│             │◄──────│  (dashboard node)   │
│ holds nar   │ block │                     │
│ serves GET  │──────►│  axum HTTP :3031    │
│ /HANDSHAKE  │ reply │  WebSocket events   │
└─────────────┘       └─────────────────────┘
                              │
                              ▼
                        ┌──────────┐
                        │ Browser  │
                        │ :3031    │
                        └──────────┘
```

## Running E2E Tests

```sh
# Block exchange tests
cargo test -p guix-p2p-e2e

# All tests including [ignored] DHT discovery (needs 3+ nodes)
cargo test -p guix-p2p-e2e -- --include-ignored --test-threads=1
```

## Running All Project Tests

```sh
cargo test --workspace
```
