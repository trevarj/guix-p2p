# Script Inventory

Runtime setup should prefer Rust binaries over shell scripts. Shell scripts in
this repository are compatibility or developer helpers.

| Script | Status | Purpose |
|--------|--------|---------|
| `scripts/guix-wrapper.sh` | compatibility shim | Execs `guix-p2p-wrapper`; persistent setup should use the Guix substitute extension through the `guix-daemon` `GUIX_EXTENSIONS_PATH` environment. |
| `scripts/test-wrapper.sh` | compatibility test shim | Runs the Rust wrapper routing unit tests. |
| `scripts/test-daemon-protocol.sh` | compatibility test shim | Runs the Rust daemon protocol and CLI contract tests. |
| `scripts/pipe-test.sh` | manual helper | Local daemon/socket smoke test with dashboard output. Keep as an ad hoc diagnostic until the E2E harness exposes an equivalent quick command. |
| `scripts/build-pages-site.sh` | CI/docs helper | Builds the static benchmark/docs Pages artifact. Candidate for Rust migration only if Pages logic grows. |
| `scripts/check-github-mirror.sh` | docs/ops helper | Compares Codeberg and GitHub `master` heads before expecting GitHub Pages to update. |
