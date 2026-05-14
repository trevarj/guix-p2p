# Multi-Peer Block Fetch Plan

## Summary

Prioritize BitTorrent-style block fetching across multiple providers after VM
benchmark phase timing is complete. The goal is to reduce the bandwidth burden
on any one seeder and improve large-NAR transfer performance when multiple
providers are available.

## Implementation

- Replace one-shot contiguous block assignment in `download_blocks_from_peers`
  with a dynamic scheduler.
- Handshake with up to `max_peers_per_download` providers.
- Track block state:
  - pending
  - in-flight
  - complete
- Track peer state:
  - available blocks
  - in-flight request count
  - failures
  - bytes received
- Keep multiple block requests in flight per peer.
- Reassign timed-out or failed blocks to another provider.
- Verify each block hash before marking it complete.
- Add config key `max_in_flight_blocks_per_peer`, default `4`.
- Use round-robin or least-in-flight assignment first.
- Defer rarest-first scheduling until benchmarks show it is needed.

## Acceptance

- Medium and large VM benchmarks with multiple seeders show block-serving
  evidence from more than one seeder.
- Failed or slow peers do not stall the whole download if other providers have
  the missing blocks.
- Existing p2p-only correctness tests continue to pass.

## Verification

- `cargo fmt`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `cargo test -p guix-p2p-e2e`
- VM benchmark with `--seed-nodes Alice,Carol,Dave` on at least one medium or
  large package.
