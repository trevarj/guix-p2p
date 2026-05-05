#!/bin/sh
# guix-p2p-substitute activation wrapper
#
# Intercepts guix-daemon's "guix substitute --query" / "--substitute"
# invocations and redirects to guix-p2p-substitute. All other guix
# subcommands pass through unchanged.
#
# Usage:
#   1. Build/install guix-p2p-substitute on PATH
#   2. Install this script as "guix" early in PATH (e.g. ~/.local/bin/guix)
#   3. Restart guix-daemon with the modified PATH
#   4. Optionally run guix-p2p-substitute --daemon as a background service
#
# The wrapper works for ALL guix commands that trigger builds and
# downloads: build, install, pull, system reconfigure, home reconfigure,
# shell, etc. Only "substitute" subcommands are intercepted.

set -eu

REAL_GUIX="/run/current-system/profile/bin/guix"

case "${1-}" in
    substitute)
        shift
        case "${1-}" in
            --query|--substitute)
                exec guix-p2p-substitute "$@"
                ;;
            *)
                exec "$REAL_GUIX" substitute "$@"
                ;;
        esac
        ;;
    *)
        exec "$REAL_GUIX" "$@"
        ;;
esac
