#!/bin/sh
# Fast dashboard demo using synthetic nars; no Guix system image or kernel build.

set -eu

PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
SEEDERS="${GUIX_P2P_DEMO_SEEDERS:-2}"
DOWNLOADERS="${GUIX_P2P_DEMO_DOWNLOADERS:-1}"
NAR_KB="${GUIX_P2P_DEMO_NAR_KB:-512}"
DASHBOARD_PORT="${GUIX_P2P_DEMO_DASHBOARD_PORT:-3031}"

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is not on PATH; run this through: guix shell -m manifest.scm -- scripts/e2e-fast-demo.sh" >&2
    exit 1
fi

cd "$PROJECT_DIR"
exec cargo run -p guix-p2p-e2e -- run \
    --seeders "$SEEDERS" \
    --downloaders "$DOWNLOADERS" \
    --nar-kb "$NAR_KB" \
    --dashboard-port "$DASHBOARD_PORT"
