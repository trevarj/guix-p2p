#!/bin/sh
# Compatibility shim for older setup docs. Prefer configuring guix-daemon's
# GUIX environment variable to point at the Rust `guix-p2p-wrapper` binary.

set -eu

exec "${GUIX_P2P_WRAPPER_BIN:-guix-p2p-wrapper}" "$@"
