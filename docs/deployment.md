# Deployment Guide

## Daemon

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

## Wrapper Flow

`guix-daemon` invokes `guix substitute --query` and
`guix substitute --substitute`. Install `scripts/guix-wrapper.sh` as `guix`
early in the daemon's `PATH` to intercept only those substitute invocations.
Other `guix` commands pass through to the real Guix binary.

Environment variables supported by the wrapper:

- `GUIX_P2P_SOCKET`: relay socket path.
- `GUIX_P2P_BIN`: `guix-p2p` binary path. Defaults to `guix-p2p` on `PATH`.
- `REAL_GUIX`: real Guix binary. Defaults to `/run/current-system/profile/bin/guix`.

Flow:

```text
guix-daemon
  -> guix substitute --query
  -> wrapper
  -> guix-p2p --query --socket <socket>
  -> daemon socket
```

Substitute mode follows the same path with `--substitute`.

## E2E VM Flow

The strict E2E proof must run Node A and Node B with separate writable Guix
stores. Node B must not already have the package seeded by Node A, so shared
host-store containers are not sufficient.

The full proof runs Node A, Node B, Node B's raw ELF `guix-daemon`, and the
client `guix build`. It runs the raw daemon binary directly, not the Guile
wrapper. It sets:

- `GUIX_STATE_DIRECTORY` to an isolated state tree.
- `GUIX_CONFIGURATION_DIRECTORY` to an isolated config tree.
- `GUIX` to a generated wrapper that forwards substitute protocol calls to
  Node B's `guix-p2p` socket.
- `GUIX_DAEMON_SOCKET` for the client `guix build`.

This forces a real `guix build <package>` through the substitute protocol
while keeping production Guix state untouched.

The current VM proof covers the full path for `hello`: two qcow2 Guix System
nodes with private stores, Node A seeding the package, Node B proving the exact
store path is absent, and Node B importing the NAR through `guix-p2p` during
`guix build --no-grafts hello`.

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
