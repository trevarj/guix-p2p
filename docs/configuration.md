# Configuration Reference

Config file:

- `$XDG_CONFIG_HOME/guix-p2p/config.toml`
- `~/.config/guix-p2p/config.toml` when `XDG_CONFIG_HOME` is unset

CLI flags in `src/main.rs` override the matching TOML values listed below.
All other tuning belongs in TOML.

## Example

```toml
listen_addr = "/ip4/0.0.0.0/udp/6881/quic-v1"
bootstrap_peers = "/ip4/bootstrap.example.org/udp/6881/quic-v1/p2p/12D3KooW..."
external_addresses = "/dns4/node.example.org/udp/6881/quic-v1"
cache_dir = "/var/cache/guix-p2p"
socket_path = "/var/cache/guix-p2p/guix-p2p.sock"
substitute_policy = "p2p-first"
substitute_urls = "https://bordeaux.guix.gnu.org,https://ci.guix.gnu.org"
min_providers = 3
dashboard_enabled = true
dashboard_port = 3030
dashboard_bind = "127.0.0.1"
seed_paths = ["/gnu/store/...-hello"]
```

## Keys

| Key | Default | CLI override | Purpose |
|-----|---------|--------------|---------|
| `bootstrap_peers` | empty | `--bootstrap-peers` | Comma-separated peer multiaddrs used for initial DHT connectivity. |
| `external_addresses` | empty | `--external-addresses` | Comma-separated listener addresses advertised to peers and provider records when autodetection is insufficient. |
| `listen_addr` | `/ip4/0.0.0.0/udp/6881/quic-v1` | `--listen-addr` | libp2p listen multiaddr. TCP and QUIC are both supported by the binary. |
| `cache_dir` | `$XDG_CACHE_HOME/guix-p2p` or `~/.cache/guix-p2p` | `--cache-dir` | Identity, nar cache, and reputation storage. |
| `substitute_urls` | `https://bordeaux.guix.gnu.org,https://ci.guix.gnu.org` | `--substitute-urls` | HTTP substitute servers used for narinfo metadata and allowed HTTP nar fallback. |
| `substitute_policy` | `p2p-first` | `--policy` | `p2p-only`, `p2p-first`, or `http-first`. |
| `block_size` | `262144` | none | Nar block size in bytes for swarm requests. |
| `request_timeout_secs` | `30` | none | Overall provider lookup and block request timeout. |
| `stall_timeout_secs` | `30` | none | Abort a P2P download after this many seconds without block progress. |
| `max_peers_per_download` | `8` | none | Upper bound on peers used for one active download. |
| `min_providers` | `3` | none | Minimum DHT providers required before attempting P2P. Local two-node tests set this to `1`. |
| `max_total_peers` | `50` | none | Connection manager peer limit. |
| `connection_retries` | `3` | none | Connection retry count. |
| `health_check_interval_secs` | `60` | none | Connection health check interval. |
| `reputation_ban_threshold` | `5` | none | Failed request threshold before a peer is treated as banned. |
| `acl_path` | `/etc/guix/acl` | none | Guix substitute signing ACL path. |
| `dashboard_enabled` | `false` | `--dashboard` | Enable the dashboard in daemon mode. |
| `dashboard_port` | `3030` | `--dashboard-port` | Dashboard HTTP port. |
| `dashboard_bind` | `127.0.0.1` | `--dashboard-bind` | Dashboard bind address. |
| `tor_socks` | unset | `--tor-socks` | SOCKS5 proxy address for Tor. |
| `tor_only` | `false` | `--tor-only` | Route network traffic only through Tor-capable paths. |
| `socket_path` | `<cache_dir>/guix-p2p.sock` | `--socket` | Unix socket used by relay mode and the Guix wrapper. |
| `seed_paths` | empty | `--seed` | Store paths serialized as raw single-item NARs and announced in the DHT. |

`bootstrap_peers` is the persisted known-peer mechanism today. Peers learned
through mDNS, identify, or normal DHT operation are added to the in-memory
routing table but are not written back to config.

Use `external_addresses` when the listen address seen locally is not the
address other peers should dial, such as port-forwarded VMs, NAT rules, or a
public DNS name.

## Policies

- `p2p-only`: fetch narinfo metadata, require enough P2P providers, and never use HTTP nar fallback.
- `p2p-first`: try P2P first, then HTTP nar fallback.
- `http-first`: try HTTP nar download first, then P2P fallback.

`substitute_urls` are still used for narinfo metadata in `p2p-only` mode. The
policy disables HTTP nar download fallback, not narinfo lookup.
