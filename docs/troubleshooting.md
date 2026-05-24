# Troubleshooting

Start by collecting the local view of the node:

```sh
guix-p2p --version
guix-p2p --doctor
curl -s http://127.0.0.1:3030/api/status
curl -s http://127.0.0.1:3030/api/diagnostics
```

If the dashboard is open, `ops -> copy diag` copies the same kind of report for
tester issues.

## Symptom Map

| Symptom | First Check |
|---------|-------------|
| `--doctor` reports `guix-integration` error | Restart `guix-daemon` after reconfigure; check the service extension. |
| Dashboard says `local-only` | Add bootstrap peers for remote tests, or accept LAN-only mDNS. |
| No peers connect | Check bootstrap multiaddrs, firewall, and whether peers include `/p2p/<peer-id>`. |
| P2P falls back to HTTP | Check transfer detail for provider lookup, handshake, and block movement. |
| `http-first` never tries P2P | This is normal unless HTTP fails; switch the fetcher to `p2p-first` for proof runs. |
| Seed action fails | Confirm dashboard bind is `127.0.0.1` and the path starts with `/gnu/store/`. |
| Package is already present | Pick another package or inspect live roots with `guix gc --referrers`. |

## No Peers Connected

Run:

```sh
guix-p2p --doctor
```

Check:

- `listen-address`, `bootstrap-peers`, and `external-addresses` do not report
  invalid multiaddrs.
- `bootstrap-peers` is not empty for remote tests.
- bootstrap peers end in `/p2p/<peer-id>` so they can seed the DHT routing
  table.
- `shareable-address` is present if other testers should dial this node.
- the router/firewall allows the listen port and transport.
- dashboard `NET` is not `local` unless this is a LAN-only test.

## No Providers Found

This means the node did not find peers advertising the requested NAR hash.

Check:

- at least one peer has seeded the store path or downloaded it successfully.
- the seeding peer is connected or reachable through the DHT.
- both peers use compatible bootstrap peers.
- `min_providers` is reasonable for the test size; local two-node tests often
  use `1`.

## HTTP Fallback Used

`p2p-first` falls back to HTTP when P2P lookup or transfer fails. This is
expected for availability, but not proof that P2P worked.

`http-first` intentionally prefers HTTP NAR download. It can still use P2P as a
fallback, but it is not the right mode for proving that one known tester can
serve a package to another tester.

Use the dashboard transfer view:

- `providers found` means DHT discovery found candidate peers.
- `blocks moved` means accepted P2P block data was observed.
- `verified/imported` means final NAR verification and import succeeded.

## Hash Mismatch

A malicious or broken peer can send bad bytes. The downloader verifies blocks
and the final NAR hash against trusted narinfo metadata. A mismatch should be
reported as a failure and should not be accepted into the store.

Include the peer id, store path, and log lines in the report.

## Dashboard Seed Action Fails

Dashboard seed mutation is allowed only when the dashboard binds to a loopback
address. This prevents remote users from mutating local seed configuration.

Use:

```sh
--dashboard-bind 127.0.0.1
```

## Relay Or Wrapper Does Not Route Guix Traffic

Check:

- `guix-p2p --daemon` is running.
- the configured `socket_path` is a Unix socket that accepts connections.
- the wrapper or Guix extension is active in the daemon environment.
- `GUIX_EXTENSIONS_PATH` includes the extension path when using the raw daemon
  integration.

`--doctor` reports whether the daemon socket is live at the time it runs. A
stale socket file is reported as an error. When no config file is present,
doctor also probes the Guix System service socket at
`/var/cache/guix-p2p/guix-p2p.sock`.

Socket-activated `guix-daemon` may not have a live process while idle. In that
case doctor checks the generated Shepherd service file for the guix-p2p
extension environment instead of requiring a running daemon.

The system relay socket must be connectable by the Guix substitute process. If
doctor reports `Permission denied`, restart the `guix-p2p` service after
upgrading to a build that sets world-connectable socket permissions.
