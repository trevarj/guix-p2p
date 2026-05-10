#!/bin/sh
# Retry the E2E VM image build and repair exact invalid Guix store paths.

set -eu

PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
VM_DIR="${GUIX_P2P_E2E_VM_DIR:-$PROJECT_DIR/target/guix-p2p-vm}"
LOG_DIR="${GUIX_P2P_E2E_LOG_DIR:-$VM_DIR/logs}"
LOOP_LOG="$LOG_DIR/e2e-vm-repair-loop.log"
MAX_REPAIRS="${GUIX_P2P_E2E_REPAIR_MAX:-1000}"
REPAIR_COMMAND="${GUIX_P2P_E2E_REPAIR_COMMAND:-sudo guix build --repair}"

timestamp() {
    date '+%Y-%m-%dT%H:%M:%S%z'
}

log() {
    mkdir -p "$LOG_DIR"
    line="$(timestamp) e2e-vm-repair-loop: $*"
    printf '%s\n' "$line" >&2
    printf '%s\n' "$line" >>"$LOOP_LOG"
}

usage() {
    cat <<EOF
Usage: scripts/e2e-vm-repair-loop.sh

Repeatedly run scripts/e2e-vm.sh rebuild-image. When the rebuild reports an
invalid host Guix store path, run:

  $REPAIR_COMMAND /gnu/store/...

Environment:
  GUIX_P2P_E2E_REPAIR_MAX      Maximum repairs before stopping, default 1000
  GUIX_P2P_E2E_REPAIR_COMMAND  Repair command prefix, default "sudo guix build --repair"
  GUIX_P2P_E2E_LOG_DIR         Log directory, default target/guix-p2p-vm/logs
EOF
}

positive_integer_or_zero() {
    case "$1" in
        '' | *[!0-9]*)
            return 1
            ;;
        *)
            return 0
            ;;
    esac
}

extract_invalid_path() {
    log_file="$1"

    sed -n "s/^.*invalid host Guix store path detected: \\(\\/gnu\\/store\\/[^[:space:]]*\\).*$/\\1/p" "$log_file" |
        tail -n 1
}

extract_builder_logs() {
    log_file="$1"

    sed -n "s/^View build log at '\\([^']*\\.drv\\.gz\\)'\\.$/\\1/p" "$log_file"
}

extract_io_error_paths() {
    build_log="$1"

    if [ ! -r "$build_log" ]; then
        log "builder log is not readable: $build_log"
        return 0
    fi

    zcat "$build_log" 2>/dev/null |
        sed -n "s/^.*i\\/o error: \\(\\/gnu\\/store\\/[^[:space:]:]*\\): No such file or directory.*$/\\1/p"
}

append_unique_path() {
    path="$1"
    path_file="$2"

    if [ -z "$path" ]; then
        return 0
    fi
    if grep -Fxq "$path" "$path_file" 2>/dev/null; then
        return 0
    fi
    printf '%s\n' "$path" >>"$path_file"
}

case "${1:-}" in
    -h | --help)
        usage
        exit 0
        ;;
    '')
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac

if ! positive_integer_or_zero "$MAX_REPAIRS"; then
    echo "GUIX_P2P_E2E_REPAIR_MAX must be a non-negative integer: $MAX_REPAIRS" >&2
    exit 2
fi

attempt=0
repair_count=0

log "starting rebuild/repair loop; max_repairs=$MAX_REPAIRS repair_command=\"$REPAIR_COMMAND\""

while :; do
    attempt="$((attempt + 1))"
    run_log="$LOG_DIR/rebuild-repair-attempt-$attempt.log"
    status_file="$LOG_DIR/rebuild-repair-attempt-$attempt.status"
    repair_paths_file="$LOG_DIR/rebuild-repair-attempt-$attempt.paths"
    : >"$repair_paths_file"

    log "attempt $attempt starting; repairs=$repair_count output=$run_log"
    set +e
    {
        "$PROJECT_DIR/scripts/e2e-vm.sh" rebuild-image
        printf '%s\n' "$?" >"$status_file"
    } 2>&1 | tee "$run_log"
    rebuild_status="$(cat "$status_file" 2>/dev/null || printf '1\n')"
    set -e

    if [ "$rebuild_status" -eq 0 ]; then
        log "attempt $attempt succeeded; repairs=$repair_count"
        exit 0
    fi

    invalid_path="$(extract_invalid_path "$run_log")"
    append_unique_path "$invalid_path" "$repair_paths_file"

    for builder_log in $(extract_builder_logs "$run_log"); do
        log "attempt $attempt scanning builder log for missing store paths; builder_log=$builder_log"
        extract_io_error_paths "$builder_log" |
            while IFS= read -r missing_path; do
                append_unique_path "$missing_path" "$repair_paths_file"
            done
    done

    if [ ! -s "$repair_paths_file" ]; then
        log "attempt $attempt failed without a recognized invalid store path; repairs=$repair_count"
        log "inspect $run_log and $LOG_DIR/image-build.log"
        exit "$rebuild_status"
    fi

    while IFS= read -r repair_path; do
        if [ "$repair_count" -ge "$MAX_REPAIRS" ]; then
            log "repair limit reached before repairing $repair_path; repairs=$repair_count max_repairs=$MAX_REPAIRS"
            exit 1
        fi

        repair_count="$((repair_count + 1))"
        repair_log="$LOG_DIR/store-repair-$repair_count.log"
        log "repair $repair_count starting; path=$repair_path output=$repair_log"
        # shellcheck disable=SC2086
        if $REPAIR_COMMAND "$repair_path" 2>&1 | tee "$repair_log"; then
            log "repair $repair_count completed; path=$repair_path"
        else
            repair_status="$?"
            log "repair $repair_count failed; status=$repair_status path=$repair_path output=$repair_log"
            exit "$repair_status"
        fi
    done <"$repair_paths_file"
done
