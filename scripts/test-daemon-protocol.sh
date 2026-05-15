#!/bin/sh
# Compatibility shim for the daemon protocol tests now covered in Rust.

set -eu

SCRIPT_DIR="$(dirname "$0")"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"
if command -v cargo >/dev/null 2>&1; then
    cargo test daemon_protocol
    cargo test cli_contract
    exec cargo test daemon::protocol::tests
fi

guix shell -m manifest.scm -- cargo test daemon_protocol
guix shell -m manifest.scm -- cargo test cli_contract
exec guix shell -m manifest.scm -- cargo test daemon::protocol::tests
