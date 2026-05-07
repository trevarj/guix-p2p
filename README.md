# guix-p2p

P2P binary substitute distribution for GNU Guix.

Sources nars from a libp2p-powered Kademlia DHT + custom block-swarm network
instead of HTTP. Zero changes to guix-daemon required.

## Quick Start

```sh
# Build
cargo build --release

# Install wrapper (no upstream Guix patch needed)
cp target/release/guix-p2p ~/.local/bin/
cp scripts/guix-wrapper.sh ~/.local/bin/guix
chmod +x ~/.local/bin/guix

# Ensure ~/.local/bin is first in daemon's PATH, then restart guix-daemon
# All substitute downloads now go through the P2P network
```

Or run the background daemon for persistent DHT participation:

```sh
guix-p2p --daemon \
    --listen-addr /ip4/0.0.0.0/udp/6881/quic-v1 \
    --cache-dir /var/cache/guix-p2p \
    --bootstrap-peers /ip4/p2p.example.org/udp/6881/quic-v1/p2p/12D3KooW...
```

## CLI

```
Usage: guix-p2p --query        # daemon --query mode (stdin protocol)
       guix-p2p --substitute    # daemon --substitute mode (stdin protocol)
       guix-p2p --daemon        # persistent background node

Global flags:
  --bootstrap-peers <CSV>   Comma-separated multiaddr list
  --listen-addr <ADDR>      Multiaddr to listen on [default: /ip4/0.0.0.0/udp/6881/quic-v1]
  --cache-dir <PATH>        Cache and identity directory
  --substitute-urls <CSV>   HTTP narinfo sources for metadata fallback
```

## How It Works

The guix-daemon spawns `guix substitute --query` and `guix substitute
--substitute` for every download. The `guix-wrapper.sh` script intercepts those
invocations and forwards them to `guix-p2p`. All other guix commands
(`guix build`, `guix install`, `guix system reconfigure`, `guix home
reconfigure`, `guix shell`, etc.) pass through unchanged.

No patches to Guix source are needed—the wrapper handles everything at the
process level.

```
guix build hello
  → guix-daemon spawns "guix substitute --query"
    → wrapper intercepts
      → guix-p2p --query
        → DHT lookup for nar hash providers
        → swarm download from peers
        → fd 4 reply: "success sha256:... 12345"
        → nar written to store
```

## Building

```sh
# All commands run from project root (Cargo workspace)
guix shell -m manifest.scm

cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt
```

## E2E Container Smoke Test

```sh
scripts/e2e-container-test.sh
```

The script defaults to TCP loopback for container compatibility. Set
`GUIX_P2P_E2E_TRANSPORT=quic` to exercise QUIC on hosts that allow UDP sockets.

## Documentation

| File | Topic |
|------|-------|
| [docs/architecture.md](docs/architecture.md) | Full architecture, data flow, activation |
| [docs/implementation-plan.md](docs/implementation-plan.md) | Phased roadmap with task status |
| [docs/dht-protocol.md](docs/dht-protocol.md) | Kademlia DHT design |
| [docs/swarm-protocol.md](docs/swarm-protocol.md) | Block exchange wire protocol |
| [docs/e2e-demo-guide.md](docs/e2e-demo-guide.md) | Interactive 2-node demo guide |

## License

GPL-3.0-or-later
