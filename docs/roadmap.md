# Roadmap

This is the active planning document. Completed phase plans and prompt files
are archived under [archive/](archive/).

## Current Status

- P2P substitute flow works through the Guix substituter protocol.
- Strict VM proof validates separate writable stores and real `guix build`
  import.
- Nodes can use a manually shared bootstrap multiaddr.
- Nodes persist recently reachable peers and reuse them on later starts.
- Dashboard package seeding, seed removal, live transfer path, catalog, peers,
  builds, and events are implemented.
- Successful downloads are cached and re-announced for re-seeding.
- VM smoke benchmark evidence exists for `hello`; the benchmark workflow now
  defaults to the full `system-build` suite for reconfigure-like evidence.
- Public network readiness is not claimed yet.

## Known Tester Readiness

The known-tester setup path is usable with a manually shared bootstrap peer,
persisted learned peers, and local dashboard access. Remaining work is mostly
operational:

- Keep setup docs aligned with the current `guix shell -m manifest.scm` plus
  Cargo build flow.
- Keep the public-readiness boundary explicit in README and deployment docs.
- Document any new dashboard mutation surfaces before enabling non-loopback
  mutation.
- Prefer explicit test-node instructions over public bootstrap promises until
  stable bootstrap infrastructure exists.

## Benchmark Work

The current benchmark evidence proves local P2P correctness, not superiority
over HTTP. Long VM benchmark suites should run through the GitHub Actions
benchmark workflow, not on developer workstations. Before making performance
claims:

- Use `system-build` as the primary benchmark target because it exercises a
  complete Guix System generation download. Keep `hello` smoke runs as workflow
  sanity checks.
- Compare `http`, `p2p-only`, `p2p-first`, and `http-first` for the same full
  system target and package tiers.
- Use repeated runs and report medians plus p95 values.
- Test small, medium, and large packages: `hello`, `git`, and `linux-libre`.
- Add multi-seeder runs with 1, 3, 5, and 8 seed nodes serving the same NAR.
- Record provider count, time to first provider, time to first block, restored
  bytes, elapsed time, and per-seeder block-serving evidence.
- Continue VM p2p-only timing work. The latest two-seeder `hello` run completed
  with local target narinfo metadata. Stale provider records are now penalized
  through handshake failure reputation and connection backoff, but larger
  repeated benchmark runs still need to confirm the latency impact.
- Compare real substitute-server conditions: normal, single-server,
  dead-primary, slow, and flaky. Record slow/flaky as skipped when OS traffic
  shaping is unavailable.
- Keep HTTP fallback usage explicit in reports for mixed-policy modes.

The detailed benchmark method lives in [benchmarks.md](benchmarks.md).

## Packaging And Release

Before a broader release:

- Keep the Guix substitute extension as the documented `guix-daemon`
  integration path. Keep `guix-p2p-wrapper` only as a compatibility fallback
  for older setups.
- Continue migrating developer-only shell helpers listed in
  [scripts.md](scripts.md) when they become part of normal setup.
- Decide whether release packages should build from tagged source snapshots
  instead of the current channel checkout.
- Decide whether release binaries are supported or source builds remain the
  only documented path.
- Tag `v0.1.7` only after packaging, bootstrap, and benchmark claims are
  documented with matching evidence.

## Bootstrap Network

Manual bootstrap multiaddrs are enough for known testers. A public network
needs:

- Stable bootstrap node infrastructure.
- Operational ownership and support expectations.
- Documented node upgrade and restart procedure.
- Clear guidance for trusted testers versus public peers.

## Full-P2P Metadata

Current P2P downloads still rely on trusted narinfo metadata from official
substitute servers or local test metadata. A future full-P2P mode is planned in
[full-p2p-attestations.md](full-p2p-attestations.md). It uses Guix
publish-style signed attestations with explicit threshold trust and keeps peer
reputation separate from build trust.

## Deferred Ideas

Deferred work remains in [future-ideas.md](future-ideas.md). The highest-value
items are:

- Proactive re-fetching for at-risk nars.
- Continued P2P-first latency tuning without duplicate HTTP downloads.
- Corporate/LAN proxy mode.
- Re-fetching missing narinfo cache entries for proactive seeding.
