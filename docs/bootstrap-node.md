# Bootstrap Node Guide

A bootstrap node is a long-running `guix-p2p --daemon` with stable identity,
stable listen address, and no required seed paths. It helps new clients join
the DHT. It does not need to be a substitute mirror.

## Paths

Recommended locations:

- Cache and identity: `/var/cache/guix-p2p`
- Socket: `/var/cache/guix-p2p/guix-p2p.sock`
- Log: `/var/log/guix-p2p/bootstrap.log`
- Config: `/etc/guix-p2p/config.toml` copied or linked into
  `$XDG_CONFIG_HOME/guix-p2p/config.toml` for the service account

Keep `cache_dir` stable. The PeerId is derived from the identity stored there.

## Operator Checklist

Before publishing a bootstrap node:

- Pick one public transport: QUIC/UDP or TCP.
- Create a stable DNS name or static public IP.
- Open the matching host firewall port.
- Forward the matching router/NAT port to the host when needed.
- Set `listen_addr` to the local bind multiaddr.
- Set `external_addresses` to the public DNS/IP multiaddr clients should dial.
- Keep `cache_dir` on persistent storage so the PeerId does not change.
- Run `guix-p2p --doctor --cache-dir /var/cache/guix-p2p`.
- Run `guix-p2p --share-info --cache-dir /var/cache/guix-p2p`.
- From another machine, run `guix-p2p --test-connectivity <published-multiaddr>`.
- Confirm dashboard `NET` is `share` or `share/no-bs`.
- Confirm the events panel shows successful peer connections, not repeated dial
  or inbound failures.
- Publish only the generated `/p2p/<peer-id>` multiaddr or `bootstrap_peers`
  snippet.

## Shepherd Service

```scheme
(define guix-p2p-bootstrap
  (make <service>
    #:provides '(guix-p2p-bootstrap)
    #:start
    (make-forkexec-constructor
     '("guix-p2p" "--daemon"
       "--listen-addr" "/ip4/0.0.0.0/udp/6881/quic-v1"
       "--cache-dir" "/var/cache/guix-p2p"
       "--socket" "/var/cache/guix-p2p/guix-p2p.sock"
       "--dashboard"
       "--dashboard-bind" "127.0.0.1"
       "--dashboard-port" "3030")
     #:environment-variables
     '("RUST_LOG=guix_p2p=info,info"
       "XDG_CONFIG_HOME=/etc")
     #:log-file "/var/log/guix-p2p/bootstrap.log")
    #:stop (make-kill-destructor)
    #:respawn? #t))
```

With `XDG_CONFIG_HOME=/etc`, the config file path is
`/etc/guix-p2p/config.toml`.

## Listen Addresses

QUIC:

```sh
guix-p2p --daemon --listen-addr /ip4/0.0.0.0/udp/6881/quic-v1
```

TCP:

```sh
guix-p2p --daemon --listen-addr /ip4/0.0.0.0/tcp/6881
```

Use TCP when UDP is unavailable or blocked. The binary supports both transports.

## Firewall

Open the selected libp2p listen port:

- QUIC: UDP `6881`
- TCP: TCP `6881`

The dashboard should usually bind to `127.0.0.1`; expose it only behind an
operator-controlled reverse proxy or tunnel.

## Publish Peer Info

Set `external_addresses` to the address clients should dial:

```toml
external_addresses = "/dns4/bootstrap.example.org/udp/6881/quic-v1"
```

Start the service and print the bootstrap bundle:

```sh
guix-p2p --share-info --cache-dir /var/cache/guix-p2p
```

The bundle includes the PeerId, derived shareable multiaddrs, and a paste-ready
client `bootstrap_peers` line. JSON output is available for scripts:

```sh
guix-p2p --share-info --json --cache-dir /var/cache/guix-p2p
curl -s http://127.0.0.1:3030/api/share-info
```

Publish the first `shareable_addresses` value exactly as clients should dial it:

```text
/ip4/203.0.113.10/udp/6881/quic-v1/p2p/12D3KooW...
/dns4/bootstrap.example.org/tcp/6881/p2p/12D3KooW...
```

## Client Config

Current project bootstrap node:

```toml
bootstrap_peers = "/dns4/guix-p2p.trevs.site/tcp/443/p2p/12D3KooWDnvPgCuPTPaMbnbLpXP7kCxmXc9F7agJPuAJWXGoDNPT"
```

```toml
bootstrap_peers = "/dns4/bootstrap.example.org/udp/6881/quic-v1/p2p/12D3KooW..."
```

Multiple bootstrap peers are comma-separated.

## Operational Checks

- `guix-p2p --doctor --cache-dir /var/cache/guix-p2p`: local readiness.
- `guix-p2p --share-info --cache-dir /var/cache/guix-p2p`: publishable peer
  bundle.
- `/api/status`: peer ID, uptime, connected peers, DHT entries.
- `/api/share-info`: same bootstrap bundle from the running dashboard daemon.
- dashboard `NET`: should be `share` when external address and bootstrap peers
  are both configured, or `share/no-bs` for the first standalone bootstrap.
- logs: look for `Swarm listening`, `Bootstrapping from`, and connection events.
- `/api/seeds`: may be empty on a pure bootstrap node.
- `/api/catalog`: grows only when the node is used in substitute query flow.

Systemd unit examples are not primary for this project; Guix System deployment
should use Shepherd.
