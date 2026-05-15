#!/bin/sh
# Compatibility shim for older setup docs. Prefer installing
# `guix-p2p-wrapper` as `guix` early in guix-daemon's PATH.

set -eu

exec "${GUIX_P2P_WRAPPER_BIN:-guix-p2p-wrapper}" "$@"
