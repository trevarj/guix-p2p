<p align="center">
  <img src="docs/assets/guix-p2p-wordmark.svg" alt="guix-p2p logo" width="360">
</p>

# guix-p2p

P2P binary substitute distribution for GNU Guix.

`guix-p2p` is a libp2p daemon and Guix substitute relay. It discovers nar
providers through Kademlia, downloads nar blocks from peers, verifies the Guix
nar hash, and can fall back to configured HTTP substitute servers when policy
allows it.

## Quick Start

```sh
guix shell -m manifest.scm
cargo build --release

target/release/guix-p2p --daemon \
    --listen-addr /ip4/0.0.0.0/udp/6881/quic-v1 \
    --cache-dir /var/cache/guix-p2p \
    --socket /var/cache/guix-p2p/guix-p2p.sock \
    --dashboard
```

Use the wrapper flow when a `guix-daemon` should route substitute queries
through the daemon:

```sh
GUIX_P2P_SOCKET=/var/cache/guix-p2p/guix-p2p.sock \
GUIX_P2P_BIN="$PWD/target/release/guix-p2p" \
REAL_GUIX="$(command -v guix)" \
scripts/guix-wrapper.sh build hello
```

## Validation

Run the real Guix smoke test:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- container-smoke --package hello --transport tcp
```

For the strict proof with separate writable stores, boot the two disposable
Guix VMs. This builds a full Guix system image and can download `linux-libre`
the first time:

```sh
cargo run -p guix-p2p-e2e -- vm image
cargo run -p guix-p2p-e2e -- vm run Alice
cargo run -p guix-p2p-e2e -- vm run Bob
```

Run local controlled benchmarks:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- benchmark --packages hello,git,emacs --iterations 3 --transport tcp
```

Benchmark CSV output is written under `target/guix-p2p-bench/`; the markdown
report is written to `docs/benchmark-results.md`.

## Documentation

| File | Topic |
|------|-------|
| [docs/architecture.md](docs/architecture.md) | Architecture, data flow, dashboard surfaces |
| [docs/configuration.md](docs/configuration.md) | TOML keys, defaults, CLI overrides |
| [docs/deployment.md](docs/deployment.md) | Daemon, relay, wrapper, and isolated Guix flow |
| [docs/bootstrap-node.md](docs/bootstrap-node.md) | Shepherd-first bootstrap node operation |
| [docs/e2e.md](docs/e2e.md) | Two-node disposable VM proof |
| [docs/benchmarks.md](docs/benchmarks.md) | Smoke and benchmark harness usage |
| [docs/dht-protocol.md](docs/dht-protocol.md) | Kademlia DHT design |
| [docs/swarm-protocol.md](docs/swarm-protocol.md) | Block exchange wire protocol |

## License

GPL-3.0-or-later
