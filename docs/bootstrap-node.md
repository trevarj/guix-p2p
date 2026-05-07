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

Start the service and read the PeerId:

```sh
curl -s http://127.0.0.1:3030/api/status
```

The response includes `peer_id`. Combine it with the public listen address:

```text
/ip4/203.0.113.10/udp/6881/quic-v1/p2p/12D3KooW...
/dns4/bootstrap.example.org/tcp/6881/p2p/12D3KooW...
```

Publish the multiaddr exactly as clients should dial it.

## Client Config

```toml
bootstrap_peers = "/dns4/bootstrap.example.org/udp/6881/quic-v1/p2p/12D3KooW..."
```

Multiple bootstrap peers are comma-separated.

## Operational Checks

- `/api/status`: peer ID, uptime, connected peers, DHT entries.
- logs: look for `Swarm listening`, `Bootstrapping from`, and connection events.
- `/api/seeds`: may be empty on a pure bootstrap node.
- `/api/catalog`: grows only when the node is used in substitute query flow.

Systemd unit examples are not primary for this project; Guix System deployment
should use Shepherd.
