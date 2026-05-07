#!/bin/sh
# guix-p2p activation wrapper
#
# Intercepts guix-daemon's "guix substitute --query" / "--substitute"
# invocations. When the daemon is running, relays through the Unix socket
# for near-instant response with a warm swarm. Falls through to direct
# invocation when the daemon is not available.
#
# Usage:
#   1. Build/install guix-p2p on PATH
#   2. Start the daemon: guix-p2p --daemon
#   3. Install this script as "guix" early in PATH (e.g. ~/.local/bin/guix)
#   4. Restart guix-daemon with the modified PATH

set -eu

REAL_GUIX="/run/current-system/profile/bin/guix"
SOCKET="${GUIX_P2P_SOCKET:-${XDG_CACHE_HOME:-$HOME/.cache}/guix-p2p/guix-p2p.sock}"

case "${1-}" in
    substitute)
        shift
        case "${1-}" in
            --query|--substitute)
                if [ -S "$SOCKET" ]; then
                    exec guix-p2p "$@" --socket "$SOCKET"
                else
                    exec "$REAL_GUIX" substitute "$@"
                fi
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