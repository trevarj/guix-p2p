#!/bin/sh
# Compatibility wrapper for the Rust E2E harness.

set -eu

PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
BASE="${GUIX_P2P_E2E_BASE:-/tmp/guix-p2p-e2e}"
PACKAGE="${GUIX_P2P_E2E_PACKAGE:-hello}"
TRANSPORT="${GUIX_P2P_E2E_TRANSPORT:-tcp}"

NODE_A_PORT="${GUIX_P2P_E2E_NODE_A_PORT:-6881}"
NODE_B_PORT="${GUIX_P2P_E2E_NODE_B_PORT:-6882}"
NODE_A_DASH="${GUIX_P2P_E2E_NODE_A_DASH:-3031}"
NODE_B_DASH="${GUIX_P2P_E2E_NODE_B_DASH:-3032}"
DASH_BIND="${GUIX_P2P_E2E_DASHBOARD_BIND:-127.0.0.1}"
HOLD="${GUIX_P2P_E2E_HOLD:-0}"

hold_arg=
if [ "$HOLD" = 1 ]; then
    hold_arg=--hold
fi

cd "$PROJECT_DIR"
exec cargo run -p guix-p2p-e2e -- container-smoke \
    --package "$PACKAGE" \
    --transport "$TRANSPORT" \
    --base "$BASE" \
    --node-a-port "$NODE_A_PORT" \
    --node-b-port "$NODE_B_PORT" \
    --node-a-dashboard-port "$NODE_A_DASH" \
    --node-b-dashboard-port "$NODE_B_DASH" \
    --dashboard-bind "$DASH_BIND" \
    $hold_arg
