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
LOG_DIR="${GUIX_P2P_E2E_LOG_DIR:-$PROJECT_DIR/target/guix-p2p-e2e-logs}"
SHELL_LOG="$LOG_DIR/e2e-container-test.log"
RUN_LOG="$LOG_DIR/e2e-container-test-output.log"
HEARTBEAT_SECS="${GUIX_P2P_E2E_HEARTBEAT_SECS:-30}"

timestamp() {
    date '+%Y-%m-%dT%H:%M:%S%z'
}

log() {
    mkdir -p "$LOG_DIR"
    line="$(timestamp) e2e-container-test: $*"
    printf '%s\n' "$line" >&2
    printf '%s\n' "$line" >>"$SHELL_LOG"
}

run_with_heartbeat() {
    label="$1"
    output_log="$2"
    shift 2

    mkdir -p "$LOG_DIR"
    rm -f "$output_log"
    log "starting $label; output=$output_log"
    started="$(date +%s)"
    "$@" >"$output_log" 2>&1 &
    pid="$!"
    log "$label pid=$pid"

    next_heartbeat="$HEARTBEAT_SECS"
    while kill -0 "$pid" 2>/dev/null; do
        sleep 1 || true
        now="$(date +%s)"
        elapsed="$((now - started))"
        if [ "$elapsed" -ge "$next_heartbeat" ] && kill -0 "$pid" 2>/dev/null; then
            log "$label still running; pid=$pid elapsed=$((now - started))s output=$output_log"
            next_heartbeat="$((next_heartbeat + HEARTBEAT_SECS))"
        fi
    done

    set +e
    wait "$pid"
    status="$?"
    set -e
    elapsed="$(($(date +%s) - started))"
    if [ "$status" -eq 0 ]; then
        log "$label completed; elapsed=${elapsed}s"
    else
        log "$label failed; status=$status elapsed=${elapsed}s output=$output_log"
    fi
    return "$status"
}

hold_arg=
if [ "$HOLD" = 1 ]; then
    hold_arg=--hold
fi

cd "$PROJECT_DIR"
log "project=$PROJECT_DIR base=$BASE"
log "package=$PACKAGE transport=$TRANSPORT node_a_port=$NODE_A_PORT node_b_port=$NODE_B_PORT"
log "dashboards=$DASH_BIND:$NODE_A_DASH,$DASH_BIND:$NODE_B_DASH hold=$HOLD"
log "shell_log=$SHELL_LOG output_log=$RUN_LOG"
run_with_heartbeat "cargo e2e container smoke" "$RUN_LOG" cargo run -p guix-p2p-e2e -- container-smoke \
    --package "$PACKAGE" \
    --transport "$TRANSPORT" \
    --base "$BASE" \
    --node-a-port "$NODE_A_PORT" \
    --node-b-port "$NODE_B_PORT" \
    --node-a-dashboard-port "$NODE_A_DASH" \
    --node-b-dashboard-port "$NODE_B_DASH" \
    --dashboard-bind "$DASH_BIND" \
    $hold_arg
