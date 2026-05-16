# Deployment Guide

## Daemon

For a persistent Guix System setup, add this repository as a Guix channel:

```scheme
(cons*
 (channel
  (name 'guix-p2p)
  (url "https://codeberg.org/trevarj/guix-p2p")
  (branch "main"))
 %default-channels)
```

After `guix pull`, import the service module from your `operating-system`
configuration:

```scheme
(use-modules (guix-p2p services))
```

From a checkout, use the manifest for contributor shells:

```sh
guix shell -m manifest.scm
```

The local package definition remains available when you want to test the
packaged binary and extension exactly as the channel package builds them:

```sh
guix shell -f guix.scm
```

The package definition imports Rust crate sources from `Cargo.lock` using
Guix's lockfile importer. Use a Guix revision that supports
`guix import crate --lockfile` and `cargo-inputs-from-lockfile`.

Run a persistent node:

```sh
guix-p2p --daemon \
  --listen-addr /ip4/0.0.0.0/udp/6881/quic-v1 \
  --cache-dir /var/cache/guix-p2p \
  --socket /var/cache/guix-p2p/guix-p2p.sock \
  --dashboard
```

The daemon owns:

- libp2p identity and DHT participation
- nar cache under `<cache_dir>/nar/`
- Unix relay socket
- dashboard API, when enabled

## Extension Flow

`guix-daemon` invokes `guix substitute --query` and
`guix substitute --substitute` normally. The recommended Guix System setup
runs the daemon with `GUIX_EXTENSIONS_PATH` containing the `guix-p2p` extension
directory, so Guix resolves the extension before its built-in substitute
command.

Put this in the `services` field of your `operating-system` configuration:

```scheme
(use-modules (guix-p2p services))

(services
  (modify-services
      (cons (service guix-p2p-service-type) %base-services)
    (guix-service-type config =>
      (guix-p2p-enable-guix-daemon-extension config))))
```

`guix-p2p-service-type` starts `guix-p2p --daemon`, adds the package to the
system profile, and creates the default cache directory. Installing the package
puts the extension at
`/run/current-system/profile/share/guix/extensions/substitute.scm`, but
`guix-daemon` only sees it when that directory is in the daemon environment.
`guix-p2p-enable-guix-daemon-extension` prepends the package extension
directory to any existing `GUIX_EXTENSIONS_PATH`; it does not discard other
configured Guix extensions. Guix Home can run user services and set login-shell
environment, but it cannot directly configure the system `guix-daemon` service
environment.

Default extension behavior:

- relay socket: `/var/cache/guix-p2p/guix-p2p.sock`
- `guix-p2p` binary: `/run/current-system/profile/bin/guix-p2p`

Optional environment overrides:

- `GUIX_EXTENSIONS_PATH`: extension directory used by Guix command lookup. The
  helper prepends `/run/current-system/profile/share/guix/extensions` to the
  daemon's existing value.
- `GUIX_P2P_SOCKET`: relay socket path.
- `GUIX_P2P_BIN`: `guix-p2p` binary path.

The legacy `scripts/guix-wrapper.sh` file is only a compatibility shim that
execs `guix-p2p-wrapper`.

Flow:

```text
guix-daemon
  -> guix substitute --query
  -> guix-p2p substitute extension
  -> guix-p2p --query --socket <socket>
  -> daemon socket
```

Substitute mode follows the same path with `--substitute`.

## E2E VM Flow

The strict E2E proof must run named VM nodes with separate writable Guix
stores. A fetcher must not already have the package seeded by another node, so
shared host-store containers are not sufficient.

The deployment proof is:

```sh
cargo run -p guix-p2p-e2e -- vm channel-proof
```

It runs a seed node, a fetch node, the fetch node's raw ELF `guix-daemon`, and
the client `guix build`. It copies the local Guix channel modules into the
fetch VM, imports `(guix-p2p services)`, calls
`guix-p2p-enable-guix-daemon-extension`, and starts the raw daemon with the
resulting environment. It sets:

- `GUIX_STATE_DIRECTORY` to an isolated state tree.
- `GUIX_CONFIGURATION_DIRECTORY` to an isolated config tree.
- `GUIX_EXTENSIONS_PATH` to include the copied `guix-p2p` substitute extension.
- `GUIX_P2P_SOCKET` to the fetch node's `guix-p2p` socket.
- `GUIX_DAEMON_SOCKET` for the client `guix build`.

This forces a real `guix build <package>` through the substitute protocol
while keeping production Guix state untouched.

The current VM proof covers the full path for `hello`: two qcow2 Guix System
nodes with private stores, one node seeding the package, another node proving
the exact store path is absent, and the fetch node importing the NAR through
`guix-p2p` during `guix build --no-grafts hello`.

## Seeding

Seed local store paths with:

```sh
guix-p2p --daemon --seed /gnu/store/...-pkg,/gnu/store/...-other
```

Each path is hashed with `guix hash -S nar -f hex`, serialized as a raw
single-item NAR with Guix's `(guix serialization) write-file`, stored under
`<cache_dir>/nar/`, and announced in the DHT. This intentionally avoids
`guix archive --export`, which produces a signed nar bundle rather than the
byte stream served by substitute servers.

## Checks

- `curl http://127.0.0.1:3030/api/status`
- `curl http://127.0.0.1:3030/api/seeds`
- `curl http://127.0.0.1:3030/api/catalog`
- logs contain `Substitute download succeeded` after a successful relay build
