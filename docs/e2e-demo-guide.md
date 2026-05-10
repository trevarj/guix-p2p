# E2E Test Network

## CLI Binary

The `guix-p2p-e2e` binary launches local test networks with web dashboards.

### `run` — synthetic test network

```sh
# Launch 2 seeders + 1 downloader, dashboards on ports 3031-3033
cargo run -p guix-p2p-e2e -- run

# Custom configuration
cargo run -p guix-p2p-e2e -- run --seeders 3 --downloaders 2 --nar-kb 512 --dashboard-port 4000
```

Seeders generate synthetic nars and announce them in the DHT. Downloaders
connect, discover providers, and request blocks. Each node gets its own
dashboard URL printed at startup.

The shell wrapper `scripts/e2e-fast-demo.sh` prints timestamped progress and
captures wrapper output under `target/guix-p2p-e2e-logs/` by default:

```sh
target/guix-p2p-e2e-logs/e2e-fast-demo.log
target/guix-p2p-e2e-logs/e2e-fast-demo-output.log
```

Set `GUIX_P2P_E2E_LOG_DIR` to move these logs, or
`GUIX_P2P_E2E_HEARTBEAT_SECS` to change the default 30-second heartbeat.

### `seed` — real store path seeding

```sh
# Seed linux and firefox from the local Guix store
cargo run -p guix-p2p-e2e -- seed --paths /gnu/store/abc-linux-6.1,/gnu/store/def-firefox-115
```

This uses `guix hash` and `guix archive --export` to compute nar hashes and
export the nar data, then starts a single seeder node with a dashboard. Other
guix-p2p nodes can connect and download the seeded packages.

Options:
- `--paths` (required): comma-separated `/gnu/store/` paths to seed
- `--dashboard-port`: port for the web dashboard (default 3031)
- `--connect`: multiaddr of a peer to dial (for downloader testing)

### What you see on the dashboard

- **Peers panel** — connected peers with reputation scores
- **Builds panel** — observed nar hashes and providers
- **Seeds panel** — locally stored nars (hash, size, blocks) with real-time
  flash animation when blocks are served to remote peers
- **Events panel** — live stream of peer connections, DHT discoveries,
  block transfers, seed additions, and blocks served
- **Header** — peer ID, connected peer count, DHT entries, seed count, uptime

## Running E2E Tests

```sh
cargo test -p guix-p2p-e2e

# Including ignored DHT discovery tests (need 3+ nodes)
cargo test -p guix-p2p-e2e -- --include-ignored --test-threads=1
```

## Running All Project Tests

```sh
cargo test --workspace
```

## Architecture

Each node runs its own libp2p QUIC swarm with Kademlia DHT and block exchange.
Seeder nodes generate synthetic nar data (or export real store paths), save it
to a `NarStore`, and announce the hash in the DHT. Downloader nodes connect to
seeders, discover providers, request blocks, and stream all events to the
dashboard via WebSocket.

The `NarStore` is shared with the dashboard so the seeds panel reflects the
current state of locally cached nars in real time.
