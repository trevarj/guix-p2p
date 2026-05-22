# Connectivity

`guix-p2p` can discover peers through configured bootstrap nodes, remembered
peer-store entries, and LAN mDNS. Remote testers should not rely on mDNS.

## Readiness Check

```sh
guix-p2p --doctor
```

Use `guix-p2p --doctor --json` when attaching diagnostics to an issue or
feeding readiness checks into automation.

The human report groups checks by substitute integration, connectivity, and
runtime state. It uses ANSI color when stdout is a terminal; set `NO_COLOR=1`
or use `--json` for plain machine-readable output.

The command checks local prerequisites:

- identity can be loaded or generated
- `listen_addr` is a valid libp2p multiaddr
- bootstrap peers are configured
- configured bootstrap peer multiaddrs are valid and include `/p2p/<peer-id>`
- configured external address multiaddrs are valid
- external addresses can produce a shareable `/p2p/<peer-id>` multiaddr
- private or loopback external addresses are flagged
- cache directory, daemon socket, ACL path, and substitute URLs are visible
- running `guix-daemon` environment appears to have the guix-p2p substitute
  extension or legacy wrapper enabled

`--doctor` does not prove that another machine can dial the node. It catches
the common local setup mistakes before testers start transferring data.

The `guix-integration` check inspects `/proc` for running `guix-daemon`
processes and looks for the service environment installed by
`guix-p2p-enable-guix-daemon-extension`: `GUIX_EXTENSIONS_PATH`,
`GUIX_P2P_BIN`, and `GUIX_P2P_SOCKET`. It also recognizes the legacy
`GUIX=.../guix-p2p-wrapper` path. Missing or unreadable integration evidence is
an error because Guix substitute traffic will not be routed through guix-p2p
unless the system `guix-daemon` has this environment.

## Bootstrap Bundle

```sh
guix-p2p --share-info
```

`--share-info` prints the local PeerId, derived shareable multiaddrs, dashboard
URL when enabled, and a paste-ready `bootstrap_peers = "..."` TOML line for
other testers.

Use JSON for scripts or issue templates:

```sh
guix-p2p --share-info --json
```

## Active Connectivity Test

```sh
guix-p2p --test-connectivity /dns4/bootstrap.example.org/udp/6881/quic-v1/p2p/12D3KooW...
```

This starts a temporary libp2p swarm and tries to dial the given multiaddr. It
exits with status `0` when the connection is established and `2` when the dial
fails. Use JSON for issue reports:

```sh
guix-p2p --test-connectivity /dns4/bootstrap.example.org/udp/6881/quic-v1/p2p/12D3KooW... --json
```

## NAT And Firewalls

For remote peers to dial a home node:

- open the selected libp2p port in the host firewall
- forward the router port to the machine running `guix-p2p`
- use the same transport in `listen_addr` and `external_addresses`
- publish a full shareable multiaddr ending in `/p2p/<peer-id>`

QUIC needs UDP:

```toml
listen_addr = "/ip4/0.0.0.0/udp/6881/quic-v1"
external_addresses = "/dns4/node.example.org/udp/6881/quic-v1"
```

TCP needs TCP:

```toml
listen_addr = "/ip4/0.0.0.0/tcp/6881"
external_addresses = "/dns4/node.example.org/tcp/6881"
```

Private addresses such as `192.168.x.x`, `10.x.x.x`, `172.16-31.x.x`, `::1`,
and `fc00::/7` are fine for LAN tests. They are not generally dialable from
the public Internet.

## Current Limits

The current rollout path assumes at least one of:

- LAN peers discover each other through mDNS.
- The node can dial a bootstrap peer.
- The node has a publicly reachable address or usable port forwarding.

Full relay/AutoNAT/DCUtR behavior is not yet treated as a tester guarantee.
Until that is implemented and surfaced in diagnostics, the dashboard `NET`
state should be read as local readiness, not a remote reachability proof.

## Dashboard Signals

The dashboard header includes:

- `CONN`: currently connected peers.
- `PEERS`: known peers with reputation or connection records.
- `DHT`: local provider records.
- `NET`: static connectivity readiness.

Open `NET` to inspect the full diagnostic report without leaving the dashboard.
Use `copy peer` to copy the same bootstrap bundle as `guix-p2p --share-info
--json`.
The events panel records outbound dial attempts, dial failures, inbound
connection attempts, inbound handshake failures, successful peer connections,
and disconnect reasons when libp2p reports one.

Use `/api/status` for machine-readable state. It includes `connectivity`,
`bootstrap_peer_count`, `shareable_addresses`, and the local PeerId.

Use `/api/share-info` for the daemon's shareable bootstrap bundle. It includes
the local PeerId, shareable multiaddrs, optional dashboard URL, diagnostic
summary flags, and a paste-ready `bootstrap_peers` config snippet.

Use `/api/diagnostics` for the dashboard daemon's full doctor-style report. It
returns the same `connectivity`, `checks`, `has_errors`, and `has_warnings`
fields as `guix-p2p --doctor --json`.

The `copy diag` dashboard button copies a compact JSON report with connectivity,
doctor checks, peer counts, DHT count, seed count, and the shareable address
state. It is meant for tester issue reports.
