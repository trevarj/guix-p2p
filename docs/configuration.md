# Configuration Reference

Config file:

- `$XDG_CONFIG_HOME/guix-p2p/config.toml`
- `~/.config/guix-p2p/config.toml` when `XDG_CONFIG_HOME` is unset

CLI flags in `src/main.rs` override the matching TOML values listed below.
All other tuning belongs in TOML.

Run local readiness checks with:

```sh
guix-p2p --doctor
```

`--doctor` loads the same config and reports bootstrap, shareable address,
cache, socket, ACL, and substitute URL readiness. It is a local setup check, not
a remote dialability proof. Add `--json` for machine-readable output.

Create the starter config with:

```sh
guix-p2p --init
```

`--init` writes the config path above and refuses to overwrite an existing file.
CLI values such as `--bootstrap-peers`, `--external-addresses`, `--listen-addr`,
`--cache-dir`, `--socket`, `--substitute-urls`, and `--policy` are copied into
the generated template.

## Example

```toml
listen_addr = "/ip4/0.0.0.0/udp/6881/quic-v1"
bootstrap_peers = "/ip4/bootstrap.example.org/udp/6881/quic-v1/p2p/12D3KooW..."
enable_default_bootstrap_peers = true
peer_store_enabled = true
peer_store_max_entries = 100
external_addresses = "/dns4/node.example.org/udp/6881/quic-v1"
cache_dir = "/var/cache/guix-p2p"
socket_path = "/var/cache/guix-p2p/guix-p2p.sock"
substitute_policy = "p2p-first"
substitute_urls = "https://bordeaux.guix.gnu.org,https://ci.guix.gnu.org"
min_providers = 3
max_upload_rate_kbps = 0
max_download_rate_kbps = 0
dashboard_enabled = true
dashboard_port = 3030
dashboard_bind = "127.0.0.1"
seed_paths = ["/gnu/store/...-hello"]
```

## Keys

| Key | Default | CLI override | Purpose |
|-----|---------|--------------|---------|
| `bootstrap_peers` | empty | `--bootstrap-peers` | Comma-separated community or private peer multiaddrs used for initial DHT connectivity. |
| `enable_default_bootstrap_peers` | `true` | none | Include built-in project bootstrap peers when they are available. The current built-in list is empty. |
| `peer_store_enabled` | `true` | none | Persist reachable peers under `cache_dir` and reuse them on later starts. |
| `peer_store_max_entries` | `100` | none | Maximum persisted peer address entries. |
| `external_addresses` | empty | `--external-addresses` | Comma-separated listener addresses advertised to peers and provider records when autodetection is insufficient. |
| `listen_addr` | `/ip4/0.0.0.0/udp/6881/quic-v1` | `--listen-addr` | libp2p listen multiaddr. TCP and QUIC are both supported by the binary. |
| `cache_dir` | `$XDG_CACHE_HOME/guix-p2p` or `~/.cache/guix-p2p` | `--cache-dir` | Identity, nar cache, and reputation storage. |
| `substitute_urls` | `https://bordeaux.guix.gnu.org,https://ci.guix.gnu.org` | `--substitute-urls` | HTTP substitute servers used for narinfo metadata and allowed HTTP nar fallback. |
| `substitute_policy` | `p2p-first` | `--policy` | `p2p-only`, `p2p-first`, or `http-first`. |
| `block_size` | `262144` | none | Nar block size in bytes for swarm requests. |
| `request_timeout_secs` | `30` | none | Provider lookup timeout and floor for the size-scaled P2P block download deadline. |
| `stall_timeout_secs` | `30` | none | Abort a P2P download after this many seconds without block progress. |
| `max_peers_per_download` | `8` | none | Upper bound on peers used for one active download. |
| `max_in_flight_blocks_per_peer` | `4` | none | Maximum outstanding block requests kept active per peer during a P2P download. |
| `max_upload_rate_kbps` | unset | none | Optional seeding upload cap in KiB/s. Unset or `0` means unlimited. |
| `max_download_rate_kbps` | unset | none | Optional HTTP fallback download cap in KiB/s. Unset or `0` means unlimited. |
| `min_providers` | `3` | `--min-providers` | Minimum DHT providers required before attempting P2P. Local two-node tests set this to `1`. |
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

Startup bootstrap peers are merged from the built-in project list, configured
`bootstrap_peers`, CLI `--bootstrap-peers`, and the persisted peer store. The
peer store is written under `cache_dir`; user config is never rewritten.

Use `external_addresses` when the listen address seen locally is not the
address other peers should dial, such as port-forwarded VMs, NAT rules, or a
public DNS name. The dashboard appends `/p2p/<peer-id>` to these addresses and
shows the shareable multiaddr for other users.

The dashboard `/api/status` response includes a `connectivity` object and
`bootstrap_peer_count`. The `NET` header condenses that state for testers:
`share`, `share/no-bs`, `client`, or `local`.

## Policies

- `p2p-only`: fetch narinfo metadata, require enough P2P providers, and never use HTTP nar fallback.
- `p2p-first`: try P2P first, then HTTP nar fallback.
- `http-first`: try HTTP nar download first, then P2P fallback.

`substitute_urls` are still used for narinfo metadata in `p2p-only` mode. The
policy disables HTTP nar download fallback, not narinfo lookup.
