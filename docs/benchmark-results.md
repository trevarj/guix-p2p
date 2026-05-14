# Benchmark Results

Current benchmark run is blocked.

## Last Attempt

- Command: `cargo run -p guix-p2p-e2e -- benchmark --packages hello --modes p2p-only --iterations 1 --transport tcp --keep-temp`
- Result: failed before throughput data was produced.
- Output CSV: `target/guix-p2p-bench/results.csv`
- Run directory: `target/guix-p2p-bench/tmp/hello-p2p-only-1`

## Blocker

The local benchmark harness now starts the P2P nodes and isolated Guix daemon,
but the p2p-only run seeds only the target output. The isolated daemon starts
with an empty dependency closure, so `guix build hello` attempts to build
dependencies from source under `--max-jobs=0` and fails before the target P2P
substitute can provide useful throughput evidence.

Do not treat benchmark data as complete until the harness either seeds the
needed closure or benchmarks a store path whose dependency closure is already
available inside the isolated daemon state.
