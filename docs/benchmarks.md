# E2E and Benchmarks

## Fast Dashboard Demo

Use this for quick demos and UI iteration:

```sh
scripts/e2e-fast-demo.sh
```

It starts a synthetic local network with two seeders, one downloader, and
dashboards on ports `3031` through `3033`. It does not build a Guix system
image and does not download `linux-libre`.

Expected dashboard/API evidence:

- each seeder dashboard shows one seeded nar under `/api/seeds`
- the downloader dashboard shows two entries under `/api/builds`
- the downloader catalog marks both synthetic nars `p2p_available = true`
- logs show handshake, block request, block serving, and block receipt

Environment overrides:

- `GUIX_P2P_DEMO_SEEDERS`
- `GUIX_P2P_DEMO_DOWNLOADERS`
- `GUIX_P2P_DEMO_NAR_KB`
- `GUIX_P2P_DEMO_DASHBOARD_PORT`

## Container Smoke

```sh
cargo run -p guix-p2p-e2e -- container-smoke --package hello --transport tcp
```

For a dashboard-first demo that keeps the validated nodes running:

```sh
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

The legacy script delegates to the same harness:

```sh
scripts/e2e-container-test.sh
```

Environment variables retained by the wrapper:

- `GUIX_P2P_E2E_BASE`
- `GUIX_P2P_E2E_PACKAGE`
- `GUIX_P2P_E2E_TRANSPORT`
- `GUIX_P2P_E2E_NODE_A_PORT`
- `GUIX_P2P_E2E_NODE_B_PORT`
- `GUIX_P2P_E2E_NODE_A_DASH`
- `GUIX_P2P_E2E_NODE_B_DASH`
- `GUIX_P2P_E2E_DASHBOARD_BIND`
- `GUIX_P2P_E2E_HOLD`

## Disposable VM

On hosts where `/gnu/store` is read-only, use the disposable Guix VM runner for
strict proof:

```sh
scripts/e2e-vm.sh run
```

The runner builds a qcow2 image, boots a writable copy under QEMU, shares the
checkout into the guest, forwards dashboard ports `3031` and `3032`, and runs
the same `container-smoke --hold` command inside the VM.

This path can download `linux-libre` because it builds a full Guix system
image. Use `scripts/e2e-fast-demo.sh` when you need a quick dashboard demo.

See [e2e-vm.md](e2e-vm.md) for the full flow.

## Benchmark

```sh
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

The smoke and benchmark harnesses require the test container to be able to
write `/gnu/store`, because raw `guix-daemon` imports substituted nars into
the store even with `--max-jobs=0`. If the host exposes `/gnu/store` read-only,
the harness fails at preflight before starting nodes. The disposable VM is the
recommended environment for the dashboard-first smoke proof. Benchmarks should
be run after the smoke proof passes there.
