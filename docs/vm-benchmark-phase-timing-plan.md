# VM Benchmark Phase Timing Plan

## Summary

Add phase-level timing to `guix-p2p-e2e vm benchmark` so benchmark reports show
where time is spent instead of only reporting one coarse elapsed value.

The benchmark must continue to use the existing VM workflow from `docs/e2e.md`:
VMs are already built, running, SSH-ready, binary-pushed, and bootstrapped.

## Implementation

- Add phase timing fields to VM benchmark records:
  - `seed_ms`
  - `prepare_ms`
  - `p2p_start_ms`
  - `provider_wait_ms`
  - `daemon_start_ms`
  - `import_ms`
  - `total_ms`
- Keep existing CSV columns stable and append phase columns at the end.
- Treat current `elapsed_ms` as total elapsed time for compatibility.
- Split the VM P2P fetch path into measured steps:
  - remove target from fetcher
  - start fetch-node `guix-p2p`
  - wait for provider visibility
  - start extension-enabled raw `guix-daemon`
  - run `guix build` import
- Split the VM HTTP path into measured steps:
  - realize dependencies and delete only the target output
  - run HTTP `guix build` import
- Seed once per package and HTTP condition when P2P modes are requested.
- Use VM seed dashboard metadata for store path, NAR hash, and NAR size.
- Update `docs/benchmark-results.md` with phase columns and phase summaries.

## Acceptance

- `vm benchmark --suite smoke --modes http,p2p-only --http-conditions normal`
  passes with non-empty phase timing columns.
- `docs/benchmark-results.md` includes total timing plus phase timing.
- Existing top-level `benchmark` output remains compatible.

## Verification

- `cargo fmt`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `cargo test -p guix-p2p-e2e`
- Manual VM smoke benchmark.
