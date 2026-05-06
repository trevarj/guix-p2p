# E2E Test Network

## CLI Binary

The `guix-p2p-e2e` binary launches local test networks with web dashboards.

```sh
# Launch 2 seeders + 1 downloader, dashboards on ports 3031-3033
cargo run -p guix-p2p-e2e -- run

# Custom configuration
cargo run -p guix-p2p-e2e -- run --seeders 3 --downloaders 2 --nar-kb 512 --dashboard-port 4000
```

Open each dashboard URL printed to stdout in a browser to see:
- **Peers panel** — connected peers with reputation scores
- **Builds panel** — observed nar hashes and providers
- **Seeds panel** — locally stored nars available for serving (nar hash, size, blocks)
- **Events panel** — real-time stream: peer connections, DHT discoveries, block transfers,
  seed additions, and blocks served to remote peers

The seeds panel shows every nar that has been saved to the local store. When a
remote peer requests blocks, a `BlockServed` event appears in the event log and the
corresponding seed row flashes green.

## Commands

| Command | Description |
|---------|-------------|
| `run` | Launch a multi-node test network with dashboards |

### `run` options

| Flag | Default | Description |
|------|---------|-------------|
| `--seeders` | 2 | Number of seeder nodes |
| `--downloaders` | 1 | Number of downloader nodes |
| `--dashboard-port` | 3031 | Starting port for dashboards |
| `--nar-kb` | 256 | Size of synthetic nars in KB |

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
Seeder nodes generate synthetic nar data, save it to a `NarStore`, and announce
the hash in the DHT. Downloader nodes connect to seeders, discover providers,
request blocks, and stream all events to the dashboard via WebSocket.

The `NarStore` is shared with the dashboard so the seeds panel reflects the
current state of locally cached nars in real time.