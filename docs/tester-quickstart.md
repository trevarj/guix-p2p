# Tester Quickstart

## System Install

For persistent Guix System testing, add the `guix-p2p` channel and service as
shown in [`deployment.md`](deployment.md), then activate the current channel
build:

```sh
guix pull
sudo guix system reconfigure /etc/config.scm
sudo herd restart guix-p2p
sudo herd restart guix-daemon
guix-p2p --doctor
```

Run this same sequence after pulling channel updates. `cargo run --bin guix-p2p
-- --doctor` uses the checkout build; `guix-p2p --doctor` uses the binary in
the active system profile. `guix-p2p --version` includes the embedded Git
commit so testers can confirm which build is active.

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

Create a starter config:

```sh
guix-p2p --init
```

This writes `$XDG_CONFIG_HOME/guix-p2p/config.toml`, or
`~/.config/guix-p2p/config.toml` when `XDG_CONFIG_HOME` is unset. It does not
overwrite an existing file.

```sh
guix-p2p --doctor --cache-dir "$GUIX_P2P_CACHE"
```

For issue reports or automation:

```sh
guix-p2p --doctor --json --cache-dir "$GUIX_P2P_CACHE"
```

Fix `error` rows before testing. `warn` rows are acceptable for LAN-only tests,
but remote peers usually need bootstrap peers and a shareable external address.

## Run A Node

Use the current project bootstrap node unless you are testing an isolated LAN
or VM setup:

```toml
bootstrap_peers = "/ip4/104.223.122.157/tcp/443/p2p/12D3KooWDnvPgCuPTPaMbnbLpXP7kCxmXc9F7agJPuAJWXGoDNPT"
```

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

To share this node with another tester, configure `external_addresses`, restart
the daemon, then run:

```sh
guix-p2p --share-info --cache-dir "$GUIX_P2P_CACHE"
```

The output includes shareable multiaddrs and a `bootstrap_peers = "..."`
snippet the other tester can paste into their config. The dashboard `copy peer`
button copies the same bundle as JSON.

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

Use [`tester-issue-template.md`](tester-issue-template.md) when filing a
tester failure.

When reporting a failure, include:

- `guix-p2p --doctor` output.
- `guix-p2p --share-info` output if this node should be dialable.
- dashboard `copy diag` output.
- dashboard `copy peer` output if this node should be dialable.
- `/api/diagnostics` output if the dashboard is reachable.
- Daemon command line and config file with private data removed.
- Dashboard `NET` state.
- Peer count, shareable address, and bootstrap peer count from `/api/status`.
- The failed store path.
- Last 100 daemon log lines around the failure.

Do not include secrets, private keys, or unrelated environment files.
