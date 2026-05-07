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

## Guix Container E2E Flow

The E2E harness runs Node A, Node B, Node B's raw ELF `guix-daemon`, and the
client `guix build` through `guix shell -CN` containers. It runs the raw daemon
binary directly, not the Guile wrapper. It sets:

- `GUIX_STATE_DIRECTORY` to an isolated state tree.
- `GUIX_CONFIGURATION_DIRECTORY` to an isolated config tree.
- `GUIX` to a generated wrapper that forwards substitute protocol calls to
  Node B's `guix-p2p` socket.
- `GUIX_DAEMON_SOCKET` for the client `guix build`.

This forces a real `guix build <package>` through the substitute protocol while
keeping production Guix state untouched.

The containers share the project checkout, generated state directory, and
`/gnu/store`. The store must be writable inside the test container so the raw
daemon can import substituted nars.

For hosts with a read-only store, use the disposable VM flow in
[e2e-vm.md](e2e-vm.md). It uses a full qcow2 image instead of `guix system vm`
so the guest has its own writable store for the proof.

## Seeding

Seed local store paths with:

```sh
guix-p2p --daemon --seed /gnu/store/...-pkg,/gnu/store/...-other
```

Each path is exported with `guix archive --export`, hashed with
`guix hash -S nar -f hex`, stored under `<cache_dir>/nar/`, and announced in
the DHT.

## Checks

- `curl http://127.0.0.1:3030/api/status`
- `curl http://127.0.0.1:3030/api/seeds`
- `curl http://127.0.0.1:3030/api/catalog`
- logs contain `Substitute download succeeded` after a successful relay build
