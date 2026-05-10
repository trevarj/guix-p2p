#!/bin/sh
# Step-by-step Guix system container diagnostics for the E2E path.

set -eu

PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
SUBSTITUTE_URLS="${GUIX_P2P_E2E_SUBSTITUTE_URLS:-https://ci.guix.gnu.org https://bordeaux.guix.gnu.org}"
ROOT_DIR="${GUIX_P2P_E2E_CONTAINER_DEBUG_DIR:-$PROJECT_DIR/target/guix-p2p-container-debug}"
LOG_DIR="$ROOT_DIR/logs"

timestamp() {
    date '+%Y-%m-%dT%H:%M:%S%z'
}

log() {
    mkdir -p "$LOG_DIR"
    printf '%s e2e-container-debug: %s\n' "$(timestamp)" "$*" >&2
}

usage() {
    cat <<EOF
Usage: scripts/e2e-container-debug.sh STEP

Steps:
  host-build        Build hello with official substitutes
  minimal-build     Build the minimal system container script
  minimal-launch    Print the sudo command for the minimal container
  pinned-build      Try the minimal container through pinned Guix time-machine
  service-build     Build the trivial-service system container script
  service-launch    Print the sudo command for the service container
  project-build     Build target/release/guix-p2p
  node-a-build      Build the Node A system container script
  node-a-launch     Print the sudo command for Node A
  node-b-build      Build the Node B system container script
  node-b-launch     Print the sudo command for Node B
  help              Show this help

Environment:
  GUIX_P2P_E2E_SUBSTITUTE_URLS          Substitute URLs, default official Guix
  GUIX_P2P_E2E_CONTAINER_DEBUG_DIR      State directory, default target/guix-p2p-container-debug
  GUIX_P2P_BOOTSTRAP_PEERS              Preserved in printed Node B launch command
EOF
}

ensure_log_dir() {
    mkdir -p "$LOG_DIR"
}

run_logged() {
    label="$1"
    output="$2"
    status_file="$output.status"
    shift 2

    ensure_log_dir
    log "starting $label; output=$output"
    rm -f "$status_file"
    set +e
    {
        "$@"
        printf '%s\n' "$?" >"$status_file"
    } 2>&1 | tee "$output"
    status="$(cat "$status_file" 2>/dev/null || printf '1\n')"
    rm -f "$status_file"
    set -e
    if [ "$status" -eq 0 ]; then
        log "$label completed"
    else
        log "$label failed; status=$status output=$output"
        exit "$status"
    fi
}

build_container() {
    name="$1"
    system_file="$2"
    root="$ROOT_DIR/$name-run-container"
    output="$LOG_DIR/$name-build.log"

    if [ -e "$root" ] || [ -L "$root" ]; then
        log "removing previous GC root link; root=$root"
        rm -f "$root"
    fi

    run_logged "$name container build" "$output" \
        guix system container -N \
        --substitute-urls="$SUBSTITUTE_URLS" \
        --root="$root" \
        "$PROJECT_DIR/$system_file"

    printf '%s\n' "$root"
}

print_launch() {
    name="$1"
    share_project="${2:-0}"
    system_file="$3"
    root="$ROOT_DIR/$name-run-container"

    if [ ! -x "$root" ]; then
        log "$name container script is missing; building it first"
        build_container "$name" "$system_file" >/dev/null
    fi

    printf 'sudo %s' "$root"
    if [ "$share_project" = 1 ]; then
        printf ' --share=%s=/src' "$PROJECT_DIR"
    fi
    printf '\n'
}

case "${1:-help}" in
    help | -h | --help)
        usage
        ;;
    host-build)
        run_logged "host hello build" "$LOG_DIR/host-build.log" \
            guix build hello --no-grafts --substitute-urls="$SUBSTITUTE_URLS"
        ;;
    minimal-build)
        build_container minimal guix/e2e-container-minimal.scm
        ;;
    minimal-launch)
        print_launch minimal 0 guix/e2e-container-minimal.scm
        ;;
    pinned-build)
        run_logged "pinned minimal container build" "$LOG_DIR/pinned-build.log" \
            guix time-machine -q \
            --url=https://codeberg.org/guix/guix.git \
            --commit=7c0cd7e45b0240b842b4f3e767599501eac42ee1 \
            -- system container -N \
            --substitute-urls="$SUBSTITUTE_URLS" \
            --root="$ROOT_DIR/pinned-minimal-run-container" \
            "$PROJECT_DIR/guix/e2e-container-minimal.scm"
        ;;
    service-build)
        build_container service guix/e2e-container-service.scm
        ;;
    service-launch)
        print_launch service 0 guix/e2e-container-service.scm
        ;;
    project-build)
        run_logged "project release build" "$LOG_DIR/project-build.log" \
            guix shell -m "$PROJECT_DIR/manifest.scm" -- cargo build --release
        ;;
    node-a-build)
        build_container node-a guix/e2e-container-node-a.scm
        ;;
    node-a-launch)
        print_launch node-a 1 guix/e2e-container-node-a.scm
        ;;
    node-b-build)
        build_container node-b guix/e2e-container-node-b.scm
        ;;
    node-b-launch)
        if [ "${GUIX_P2P_BOOTSTRAP_PEERS:-}" ]; then
            printf 'sudo env GUIX_P2P_BOOTSTRAP_PEERS=%s ' "'$GUIX_P2P_BOOTSTRAP_PEERS'"
            print_launch node-b 1 guix/e2e-container-node-b.scm | sed 's/^sudo //'
        else
            print_launch node-b 1 guix/e2e-container-node-b.scm
        fi
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
