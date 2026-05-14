# E2E and Benchmarks

## Container Smoke

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- container-smoke --package hello --transport tcp
```

For a dashboard-first demo that keeps the validated nodes running:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- container-smoke \
  --package hello \
  --transport tcp \
  --dashboard-bind 0.0.0.0 \
  --hold \
  --keep-temp
```

Defaults:

- package: `hello`
- transport: TCP loopback
- base: `/tmp/guix-p2p-e2e`
- Node A dashboard: `3031`
- Node B dashboard: `3032`
- dashboard bind: `127.0.0.1`

Use QUIC on hosts that allow UDP:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- container-smoke --package hello --transport quic
```

The harness:

- builds or locates `target/release/guix-p2p`
- resolves a real store path with `guix build <package>`
- resolves the raw ELF `guix-daemon`
- preflights `guix shell -CN` with writable `/gnu/store`
- starts Node A in a Guix container with the resolved store path seeded
- starts Node B in a separate Guix container with `substitute_policy = "p2p-only"` and `min_providers = 1`
- starts an isolated raw `guix-daemon` in Node B's Guix container with `GUIX` pointing at a generated wrapper
- runs `guix build <package>` in a Guix container through Node B's daemon socket
- captures logs under `$BASE/logs/`
- with `--hold`, keeps daemons and dashboards alive after validation until Ctrl-C

Acceptance checks:

- `guix build` exits successfully.
- Node A `/api/seeds` includes the seeded nar.
- Node B `/api/catalog` includes the requested store path or nar hash.
- Node A logs show block serving.
- Node B logs show p2p-only handling and a successful substitute download.
- Node B logs do not show HTTP nar fallback in p2p-only mode.

## E2E VM Proof

The strict proof requires separate writable stores so a fetcher can prove it
does not already have the package seeded by another node. Shared host-store
containers are not a valid full proof for that requirement.

Use `cargo run -p guix-p2p-e2e -- vm ...` for the strict named-node VM proof
with separate writable stores. The VM proof currently passes for `hello`
through a fetcher node's raw `guix-daemon` wrapper path and verifies that the
imported store path is a restored directory.

## Benchmark

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- benchmark --packages hello,git,emacs --iterations 3 --transport tcp
```

Defaults:

- packages: `hello,git,emacs`
- modes: `http,p2p-only,p2p-first`
- iterations: `3`
- output directory: `target/guix-p2p-bench/`

Modes:

- `http`: isolated raw `guix-daemon` without the P2P wrapper.
- `p2p-only`: Node A seeded, Node B p2p-only, `min_providers = 1`.
- `p2p-first`: Node A seeded, Node B p2p-first, `min_providers = 1`.

Outputs:

- `target/guix-p2p-bench/results.csv`
- `docs/benchmark-results.md`

The report includes host and Rust summary, package store paths, nar hashes,
nar sizes when observed from dashboard seed data, per-run elapsed time,
medians, and whether P2P block-serving evidence was observed.

Per-run temp directories are removed unless `--keep-temp` is passed.

For p2p modes, the container benchmark also runs an explicit relay substitute
restore because shared host-store containers can make exact store-path builds a
no-op. With `--keep-temp`, inspect:

- `$BASE/tmp/<package>-<mode>-<iteration>/manual-substitute-output`
- `$BASE/tmp/<package>-<mode>-<iteration>/logs/direct-substitute.log`
- `$BASE/tmp/<package>-<mode>-<iteration>/logs/node-a.log`
- `$BASE/tmp/<package>-<mode>-<iteration>/logs/node-b.log`

If a kept temp directory contains container-owned files, rerun with a fresh
`--base` rather than deleting the evidence directory.

The smoke and benchmark harnesses require the test container to be able to
write `/gnu/store`, because raw `guix-daemon` imports substituted nars into
the store even with `--max-jobs=0`. If the host exposes `/gnu/store` read-only,
the harness fails at preflight before starting nodes. The disposable VM proof
is the authoritative full-store-isolation check; the benchmark harness remains
the faster controlled timing tool.

## Future Benchmark Work

The current `hello` result proves the local p2p-only substitute path and block
transfer. It does not prove that P2P is faster than HTTP. Future benchmark work
should compare HTTP and P2P under repeated, controlled scenarios.

Baseline comparison:

- Run `http`, `p2p-only`, and `p2p-first` for the same package set.
- Use a fresh `--base` per run group so kept temp state does not contaminate
  results.
- Use at least three iterations and compare medians, not single runs.
- Record substitute URLs, host Guix revision, transport, package store paths,
  NAR hashes, and NAR sizes.

Package set:

- `hello`: small correctness and harness sanity check.
- `git`: medium package with non-trivial closure and transfer size.
- `emacs` or another large already-realized package: useful for bandwidth and
  multi-peer behavior.

Multi-peer P2P comparison:

- Use one fetch node, one neutral bootstrap node, and N seed nodes all seeding
  the same desired NAR.
- Run seed counts of 1, 3, 5, and 8 for the same package and transport.
- Record provider count, time to first provider, time to first block, total
  restored bytes, elapsed time, and per-seeder block-serving evidence.
- Keep the explicit relay substitute restore for p2p modes; shared host-store
  containers can make exact store-path builds no-op.

Mixed-policy comparison:

- Compare `p2p-first` and `http-first` when P2P providers are available.
- Repeat with slow or bandwidth-limited seeders once upload limits are wired
  into block serving.
- Repeat with no providers to measure HTTP fallback latency and confirm the
  fallback path is visible in logs.

Acceptance criteria for publishing a benchmark claim:

- Every p2p run has block-serving evidence in seed-node logs.
- Every restored output exists under the kept run directory.
- HTTP fallback usage is explicitly recorded for `p2p-first` and `http-first`.
- Report median and p95 elapsed time for each package/mode/seed-count group.
