#!/bin/sh
# Deprecated compatibility wrapper. Use scripts/e2e.sh.

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
printf '%s\n' "warning: scripts/e2e-private-store.sh is deprecated; use scripts/e2e.sh" >&2
exec "$SCRIPT_DIR/e2e.sh" "$@"
