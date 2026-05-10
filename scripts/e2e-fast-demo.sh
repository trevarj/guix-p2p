#!/bin/sh
# Fast dashboard demo using synthetic nars; no Guix system image or kernel build.

set -eu

PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
SEEDERS="${GUIX_P2P_DEMO_SEEDERS:-2}"
DOWNLOADERS="${GUIX_P2P_DEMO_DOWNLOADERS:-1}"
NAR_KB="${GUIX_P2P_DEMO_NAR_KB:-512}"
DASHBOARD_PORT="${GUIX_P2P_DEMO_DASHBOARD_PORT:-3031}"
LOG_DIR="${GUIX_P2P_E2E_LOG_DIR:-$PROJECT_DIR/target/guix-p2p-e2e-logs}"
SHELL_LOG="$LOG_DIR/e2e-fast-demo.log"
RUN_LOG="$LOG_DIR/e2e-fast-demo-output.log"
HEARTBEAT_SECS="${GUIX_P2P_E2E_HEARTBEAT_SECS:-30}"

timestamp() {
    date '+%Y-%m-%dT%H:%M:%S%z'
}

log() {
    mkdir -p "$LOG_DIR"
    line="$(timestamp) e2e-fast-demo: $*"
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

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is not on PATH; run this through: guix shell -m manifest.scm -- scripts/e2e-fast-demo.sh" >&2
    exit 1
fi

cd "$PROJECT_DIR"
log "project=$PROJECT_DIR"
log "seeders=$SEEDERS downloaders=$DOWNLOADERS nar_kb=$NAR_KB dashboard_port=$DASHBOARD_PORT"
log "shell_log=$SHELL_LOG output_log=$RUN_LOG"
run_with_heartbeat "cargo e2e fast demo" "$RUN_LOG" cargo run -p guix-p2p-e2e -- run \
    --seeders "$SEEDERS" \
    --downloaders "$DOWNLOADERS" \
    --nar-kb "$NAR_KB" \
    --dashboard-port "$DASHBOARD_PORT"
