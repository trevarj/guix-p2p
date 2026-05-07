# Benchmark Results

Run the benchmark harness to generate current local results:

```sh
cargo run -p guix-p2p-e2e -- benchmark --packages hello,git,emacs --iterations 3 --transport tcp
```

The harness overwrites this file and writes machine-readable CSV to
`target/guix-p2p-bench/results.csv`.
