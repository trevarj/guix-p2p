<p align="center">
  <img src="docs/assets/guix-p2p-wordmark.svg" alt="guix-p2p logo" width="360">
</p>

# guix-p2p

P2P binary substitute distribution for GNU Guix.

`guix-p2p` is a libp2p daemon and Guix substitute relay. It discovers nar
providers through Kademlia, downloads nar blocks from peers, verifies the Guix
nar hash, and can fall back to configured HTTP substitute servers when policy
allows it.

## How It Works

`guix-daemon` already talks to substituters through the `guix substitute`
protocol. `guix-p2p` keeps that protocol intact: a small wrapper intercepts
`guix substitute --query` and `guix substitute --substitute`, then forwards
those requests to a long-running `guix-p2p --daemon` over a Unix socket.

The daemon answers `have` and `info` queries only when the requested store path
has trusted narinfo metadata and enough P2P providers. For `substitute`
requests, it downloads the NAR from peers, verifies the official Guix nar hash,
and writes the result back through the same file descriptor that
`guix-daemon` expects from any substituter.

```text
guix build hello
      |
      v
guix-daemon
      |
      | spawns: guix substitute --query / --substitute
      v
wrapper or patched guix substitute
      |
      | forwards stdin/fd 4 over Unix socket
      v
guix-p2p --daemon
      |
      +-- fetch signed narinfo from official substitute URLs
      +-- find providers in the libp2p Kademlia DHT
      +-- download NAR blocks from peers
      +-- verify nar hash
      |
      v
reply to guix-daemon as a normal substitute
```

## Quick Start

```sh
guix shell -m manifest.scm
cargo build --release

target/release/guix-p2p --daemon \
    --listen-addr /ip4/0.0.0.0/udp/6881/quic-v1 \
    --dashboard
```

Use the wrapper flow when a `guix-daemon` should route substitute queries
through the daemon:

```sh
GUIX_P2P_SOCKET="${XDG_CACHE_HOME:-$HOME/.cache}/guix-p2p/guix-p2p.sock" \
GUIX_P2P_BIN="$PWD/target/release/guix-p2p" \
REAL_GUIX="$(command -v guix)" \
target/release/guix-p2p-wrapper build hello
```

## Setup For Known Testers

Build from a development shell:

```sh
guix shell -m manifest.scm
cargo build --release
```

Start a daemon with the default cache directory, relay socket, and dashboard:

```sh
target/release/guix-p2p --daemon \
    --listen-addr /ip4/0.0.0.0/udp/6881/quic-v1 \
    --dashboard \
    --dashboard-bind 127.0.0.1 \
    --dashboard-port 3030
```

If another tester is acting as a bootstrap node, ask them for their full
multiaddr and pass it as `bootstrap_peers` in `~/.config/guix-p2p/config.toml`:

```toml
bootstrap_peers = "/ip4/203.0.113.10/udp/6881/quic-v1/p2p/12D3KooW..."
```

Use the wrapper flow to route a local Guix build through the daemon:

```sh
GUIX_P2P_SOCKET="${XDG_CACHE_HOME:-$HOME/.cache}/guix-p2p/guix-p2p.sock" \
GUIX_P2P_BIN="$PWD/target/release/guix-p2p" \
REAL_GUIX="$(command -v guix)" \
target/release/guix-p2p-wrapper build hello
```

The maintained strict validation flow is documented in
[docs/e2e.md](docs/e2e.md). It uses disposable Guix System VMs with separate
stores and proves the substitute import path end to end.

Known public-network gaps:

- Stable bootstrap infrastructure is not provided yet.
- Guix packaging and channel distribution are not finished.
- Remote dashboard mutation is not authenticated.
- Operational support expectations are still for known testers, not a public
  network.

## Validation

The strict end-to-end proof uses disposable Guix System VMs with separate
writable stores. Follow [docs/e2e.md](docs/e2e.md) for the maintained command
sequence.

For a faster smoke test that exercises the Guix substituter path without the
full VM proof:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- container-smoke --package hello --transport tcp
```

Run local controlled benchmarks:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- benchmark --suite smoke --iterations 1 --transport tcp
```

Benchmark CSV output is written under `target/guix-p2p-bench/`; the markdown
report is written to `docs/benchmark-results.md`. Use `--suite standard` for
small, medium, and large package tiers.

## Documentation

| File | Topic |
|------|-------|
| [docs/architecture.md](docs/architecture.md) | Architecture, data flow, dashboard surfaces |
| [docs/configuration.md](docs/configuration.md) | TOML keys, defaults, CLI overrides |
| [docs/deployment.md](docs/deployment.md) | Daemon, relay, wrapper, and isolated Guix flow |
| [docs/scripts.md](docs/scripts.md) | Script inventory and Rust migration status |
| [docs/bootstrap-node.md](docs/bootstrap-node.md) | Shepherd-first bootstrap node operation |
| [docs/e2e.md](docs/e2e.md) | Two-node disposable VM proof |
| [docs/benchmarks.md](docs/benchmarks.md) | Smoke and benchmark harness usage |
| [docs/mirror.md](docs/mirror.md) | Codeberg-to-GitHub mirror and GitHub CI setup |
| [docs/dht-protocol.md](docs/dht-protocol.md) | Kademlia DHT design |
| [docs/swarm-protocol.md](docs/swarm-protocol.md) | Block exchange wire protocol |
| [docs/roadmap.md](docs/roadmap.md) | Current remaining work |
| [docs/README.md](docs/README.md) | Full documentation index |

## License

GPL-3.0-or-later
