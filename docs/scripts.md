# Script Inventory

Runtime setup should prefer Rust binaries over shell scripts. Shell scripts in
this repository are compatibility or developer helpers.

| Script | Status | Purpose |
|--------|--------|---------|
| `scripts/guix-wrapper.sh` | compatibility shim | Execs `guix-p2p-wrapper`; install the Rust binary as `guix` for normal use. |
| `scripts/test-wrapper.sh` | compatibility test shim | Runs the Rust wrapper routing unit tests. |
| `scripts/pipe-test.sh` | developer helper | Manual local daemon/socket protocol smoke test. Keep until covered by a Rust harness. |
| `scripts/test-daemon-protocol.sh` | developer helper | Older fd 4 protocol smoke test. Candidate for Rust integration-test migration. |
| `scripts/build-pages-site.sh` | CI/docs helper | Builds the static benchmark/docs Pages artifact. Candidate for Rust migration only if Pages logic grows. |
