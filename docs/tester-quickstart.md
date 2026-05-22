# Tester Quickstart

## Build

```sh
cargo build --release
```

Use a stable cache directory while testing. The peer identity is stored there,
so changing it changes the PeerId.

```sh
export GUIX_P2P_CACHE="$HOME/.cache/guix-p2p"
```

## Check Readiness

```sh
guix-p2p --doctor --cache-dir "$GUIX_P2P_CACHE"
```

Fix `error` rows before testing. `warn` rows are acceptable for LAN-only tests,
but remote peers usually need bootstrap peers and a shareable external address.

## Run A Node

```sh
guix-p2p --daemon \
  --cache-dir "$GUIX_P2P_CACHE" \
  --listen-addr /ip4/0.0.0.0/udp/6881/quic-v1 \
  --bootstrap-peers /dns4/bootstrap.example.org/udp/6881/quic-v1/p2p/12D3KooW... \
  --dashboard \
  --dashboard-bind 127.0.0.1 \
  --dashboard-port 3030
```

Open the dashboard:

```text
http://127.0.0.1:3030
```

The `NET` header shows local connectivity readiness:

- `share`: bootstrap peers and shareable addresses are configured.
- `share/no-bs`: this node is shareable, but has no bootstrap peers.
- `client`: this node can dial bootstrap peers, but is not advertising a
  shareable address.
- `local`: no bootstrap peers or shareable address; LAN mDNS may still work.

## Seed A Store Path

From the dashboard, use the package list and press `seed`, or start the daemon
with explicit seed paths:

```sh
guix-p2p --daemon --seed /gnu/store/...-hello
```

The dashboard `active seeds` panel shows NARs this node can serve.

## Useful Failure Report

When reporting a failure, include:

- `guix-p2p --doctor` output.
- dashboard `copy diag` output.
- Daemon command line and config file with private data removed.
- Dashboard `NET` state.
- Peer count, shareable address, and bootstrap peer count from `/api/status`.
- The failed store path.
- Last 100 daemon log lines around the failure.

Do not include secrets, private keys, or unrelated environment files.
