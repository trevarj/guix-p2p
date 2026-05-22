# Connectivity

`guix-p2p` can discover peers through configured bootstrap nodes, remembered
peer-store entries, and LAN mDNS. Remote testers should not rely on mDNS.

## Readiness Check

```sh
guix-p2p --doctor
```

Use `guix-p2p --doctor --json` when attaching diagnostics to an issue or
feeding readiness checks into automation.

The command checks local prerequisites:

- identity can be loaded or generated
- bootstrap peers are configured
- external addresses can produce a shareable `/p2p/<peer-id>` multiaddr
- private or loopback external addresses are flagged
- cache directory, daemon socket, ACL path, and substitute URLs are visible

`--doctor` does not prove that another machine can dial the node. It catches
the common local setup mistakes before testers start transferring data.

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

Use `/api/status` for machine-readable state. It includes `connectivity`,
`bootstrap_peer_count`, `shareable_addresses`, and the local PeerId.

The `copy diag` dashboard button copies a compact JSON report with connectivity,
peer counts, DHT count, seed count, and the shareable address state. It is meant
for tester issue reports.
