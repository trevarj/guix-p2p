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
protocol. `guix-p2p` keeps that protocol intact: the daemon still invokes
`guix substitute --query` and `guix substitute --substitute` normally. The
integration point is the daemon environment: `GUIX_EXTENSIONS_PATH` must include
the `guix-p2p` extension directory. When that is true, Guix resolves the
`substitute` command from the extension first, and the extension forwards
substitute protocol requests to a long-running `guix-p2p --daemon` over a Unix
socket.

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
guix-p2p substitute extension from GUIX_EXTENSIONS_PATH
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

For persistent Guix System usage, add this repository as a Guix channel:

```scheme
(cons*
 (channel
  (name 'guix-p2p)
  (url "https://codeberg.org/trevarj/guix-p2p")
  (branch "main"))
 %default-channels)
```

After `guix pull`, import the service module in your `operating-system`
configuration and configure the system daemon environment with
`guix-p2p-enable-guix-daemon-extension`:

```scheme
(use-modules (guix-p2p services))

(services
  (modify-services
      (cons (service guix-p2p-service-type) %base-services)
    (guix-service-type config =>
      (guix-p2p-enable-guix-daemon-extension config))))
```

The service starts `guix-p2p --daemon`, adds the package to the system profile,
and configures `guix-daemon` to resolve the `guix-p2p` substitute extension
through `GUIX_EXTENSIONS_PATH`. The helper prepends the package extension
directory to any existing `GUIX_EXTENSIONS_PATH`; it does not replace or
discard other Guix extensions. Ordinary Guix commands are unchanged; only the
internal substitute protocol calls that `guix-daemon` makes during a build or
reconfigure are intercepted.

After reconfiguring, keep using normal Guix commands:

```sh
guix build hello
sudo guix system reconfigure /etc/config.scm
```

The extension defaults to:

- relay socket: `/var/cache/guix-p2p/guix-p2p.sock`
- `guix-p2p` binary: `/run/current-system/profile/bin/guix-p2p`

`guix-p2p-wrapper` remains installed for compatibility with older setups.

See [docs/deployment.md](docs/deployment.md) for persistent service details.

## Local Development

From a checkout, use the manifest for the contributor shell. It includes Rust,
Cargo, C toolchain packages, certificates, and local test helpers:

```sh
guix shell -m manifest.scm
```

The local package definition remains available when you want to test the
packaged binary and extension exactly as the channel package builds them:

```sh
guix shell -f guix.scm
```

Start a daemon with the default cache directory, relay socket, and local
dashboard from the shell:

```sh
guix-p2p --daemon \
    --listen-addr /ip4/0.0.0.0/udp/6881/quic-v1 \
    --dashboard \
    --dashboard-bind 127.0.0.1 \
    --dashboard-port 3030
```

To build the local package directly:

```sh
guix build -f guix.scm
```

The package definition uses Guix's Rust lockfile importer to build from
`Cargo.lock`, so it requires a Guix revision with `guix import crate --lockfile`
and `cargo-inputs-from-lockfile` support.

## Bootstrap Peers

If another tester is acting as a bootstrap node, ask them for their full
multiaddr and pass it as `bootstrap_peers` in `~/.config/guix-p2p/config.toml`:

```toml
bootstrap_peers = "/ip4/203.0.113.10/udp/6881/quic-v1/p2p/12D3KooW..."
```

The dashboard shows the shareable multiaddr for this node when
`external_addresses` is configured. Set it to the address other peers can dial:

```toml
external_addresses = "/dns4/node.example.org/udp/6881/quic-v1"
```

Then open the dashboard and use the copy control next to the PeerId. The value
has this form:

```text
/dns4/node.example.org/udp/6881/quic-v1/p2p/12D3KooW...
```

Do not share `/ip4/0.0.0.0/...`; that is only a local bind address.

## Validation

Run the Rust test suite:

```sh
cargo test
cargo test -p guix-p2p-e2e
```

The strict end-to-end deployment proof uses disposable Guix System VMs with
separate writable stores and exercises the Guix channel service module:

```sh
cargo run -p guix-p2p-e2e -- vm channel-proof
```

Follow [docs/e2e.md](docs/e2e.md) for the maintained VM setup sequence. A
faster container smoke test is also available:

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
| [docs/deployment.md](docs/deployment.md) | Daemon, relay, extension, and isolated Guix flow |
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
