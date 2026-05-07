# E2E and Benchmarks

## Container Smoke

```sh
cargo run -p guix-p2p-e2e -- container-smoke --package hello --transport tcp
```

Defaults:

- package: `hello`
- transport: TCP loopback
- base: `/tmp/guix-p2p-e2e`
- Node A dashboard: `3031`
- Node B dashboard: `3032`

Use QUIC on hosts that allow UDP:

```sh
cargo run -p guix-p2p-e2e -- container-smoke --package hello --transport quic
```

The harness:

- builds or locates `target/release/guix-p2p`
- resolves a real store path with `guix build <package>`
- resolves the raw ELF `guix-daemon`
- starts Node A with the resolved store path seeded
- starts Node B with `substitute_policy = "p2p-only"` and `min_providers = 1`
- starts an isolated raw `guix-daemon` with `GUIX` pointing at a generated wrapper
- runs `guix build <package>` through Node B's daemon socket
- captures logs under `$BASE/logs/`

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
