# Roadmap

This is the active planning document. Completed phase plans and prompt files
are archived under [archive/](archive/).

## Current Status

- P2P substitute flow works through the Guix substituter protocol.
- Strict VM proof validates separate writable stores and real `guix build`
  import.
- Nodes can use a manually shared bootstrap multiaddr.
- Dashboard package seeding, seed removal, live transfer path, catalog, peers,
  builds, and events are implemented.
- Successful downloads are cached and re-announced for re-seeding.
- Local benchmark evidence exists for a minimal p2p-only `hello` substitute
  path.
- Public network readiness is not claimed yet.

## Known Tester Readiness

The known-tester setup path is usable with a manually shared bootstrap peer and
local dashboard access. Remaining work is mostly operational:

- Keep setup docs aligned with the current `guix shell -m manifest.scm` plus
  Cargo build flow.
- Keep the public-readiness boundary explicit in README and deployment docs.
- Document any new dashboard mutation surfaces before enabling non-loopback
  mutation.
- Prefer explicit test-node instructions over public bootstrap promises until
  stable bootstrap infrastructure exists.

## Benchmark Work

The current benchmark evidence proves local P2P correctness, not superiority
over HTTP. Before making performance claims:

- Compare `http`, `p2p-only`, `p2p-first`, and `http-first` for the same
  package tiers.
- Use repeated runs and report medians plus p95 values.
- Test small, medium, and large packages: `hello`, `git`, and `linux-libre`.
- Add multi-seeder runs with 1, 3, 5, and 8 seed nodes serving the same NAR.
- Record provider count, time to first provider, time to first block, restored
  bytes, elapsed time, and per-seeder block-serving evidence.
- Continue VM p2p-only timing work. The latest two-seeder `hello` run completed
  with local target narinfo metadata, but stale provider records can still add
  dial failures and extra transfer latency.
- Compare real substitute-server conditions: normal, single-server,
  dead-primary, slow, and flaky. Record slow/flaky as skipped when OS traffic
  shaping is unavailable.
- Keep HTTP fallback usage explicit in reports for mixed-policy modes.

The detailed benchmark method lives in [benchmarks.md](benchmarks.md).

## Packaging And Release

Before a broader release:

- Add a Guix channel package definition.
- Document Guix channel installation once packaging exists.
- Decide whether release binaries are supported or source builds remain the
  only documented path.
- Tag `v0.1.0` only after packaging, bootstrap, and benchmark claims are
  documented with matching evidence.

## Bootstrap Network

Manual bootstrap multiaddrs are enough for known testers. A public network
needs:

- Stable bootstrap node infrastructure.
- Operational ownership and support expectations.
- Documented node upgrade and restart procedure.
- Clear guidance for trusted testers versus public peers.

## Deferred Ideas

Deferred work remains in [future-ideas.md](future-ideas.md). The highest-value
items are:

- Background health monitoring for provider counts.
- Parallel HTTP plus P2P fetching.
- Upload bandwidth limits for seeders.
- Corporate/LAN proxy mode.
- Re-seeding from the narinfo cache on startup.
