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
guix-p2p-wrapper installed as guix in PATH
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

The local package definition installs both `guix-p2p` and
`guix-p2p-wrapper`:

```sh
guix shell -f guix.scm
```

Start a daemon with the default cache directory, relay socket, and local
dashboard from that shell:

```sh
guix-p2p --daemon \
    --listen-addr /ip4/0.0.0.0/udp/6881/quic-v1 \
    --dashboard \
    --dashboard-bind 127.0.0.1 \
    --dashboard-port 3030
```

Install `guix-p2p-wrapper` as the `guix` command earlier in the `guix-daemon`
service `PATH` than the real Guix binary. The wrapper passes ordinary Guix
commands through unchanged, but intercepts the substitute protocol calls that
`guix-daemon` makes during a build or reconfigure. With the daemon configured
for that wrapper path, keep using normal Guix commands:

```sh
guix build hello
sudo guix system reconfigure /etc/config.scm
```

The wrapper defaults to:

- relay socket: `${XDG_CACHE_HOME:-$HOME/.cache}/guix-p2p/guix-p2p.sock`
- `guix-p2p` binary: `guix-p2p` from `PATH`
- real Guix binary: `/run/current-system/profile/bin/guix`

See [docs/deployment.md](docs/deployment.md) for persistent service and wrapper
installation options. To add the local package to a Guix profile, use:

```sh
guix package -f guix.scm
```

Note: the package definition currently builds from Cargo's locked dependency
graph. A fully offline Guix build still requires importing or vendoring the Rust
crate dependencies.

## Bootstrap Peers

If another tester is acting as a bootstrap node, ask them for their full
multiaddr and pass it as `bootstrap_peers` in `~/.config/guix-p2p/config.toml`:

```toml
bootstrap_peers = "/ip4/203.0.113.10/udp/6881/quic-v1/p2p/12D3KooW..."
```

## Validation

Run the Rust test suite:

```sh
cargo test
cargo test -p guix-p2p-e2e
```

The strict end-to-end proof uses disposable Guix System VMs with separate
writable stores. Follow [docs/e2e.md](docs/e2e.md) for the maintained command
sequence. A faster container smoke test is also available:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- container-smoke --package hello --transport tcp
```

Benchmarks run in CI. Use the
[GitHub Actions benchmark workflow](https://github.com/trevarj/guix-p2p/actions/workflows/benchmarks.yml)
and [docs/benchmarks.md](docs/benchmarks.md) for benchmark runs and artifacts.

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
