#!/bin/sh
# Build and launch the private-store VM proof scaffolding.

set -eu

PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
STATE_DIR="${GUIX_P2P_E2E_PRIVATE_DIR:-$PROJECT_DIR/target/guix-p2p-private-store}"
IMAGE_SIZE="${GUIX_P2P_E2E_IMAGE_SIZE:-8G}"
MEMORY="${GUIX_P2P_E2E_VM_MEMORY:-2048}"
CPUS="${GUIX_P2P_E2E_VM_CPUS:-2}"
SUBSTITUTE_URLS="${GUIX_P2P_E2E_SUBSTITUTE_URLS:-https://ci.guix.gnu.org https://bordeaux.guix.gnu.org}"
NODE_A_SYSTEM="$PROJECT_DIR/guix/e2e-private-node-a.scm"
NODE_B_SYSTEM="$PROJECT_DIR/guix/e2e-private-node-b.scm"

timestamp() {
    date '+%Y-%m-%dT%H:%M:%S%z'
}

log() {
    mkdir -p "$STATE_DIR/logs"
    printf '%s e2e-private-store: %s\n' "$(timestamp)" "$*" >&2
}

run_logged() {
    label="$1"
    output="$2"
    status_file="$output.status"
    shift 2

    mkdir -p "$STATE_DIR/logs"
    rm -f "$status_file"
    log "starting $label; output=$output"
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

usage() {
    cat <<EOF
Usage: scripts/e2e-private-store.sh STEP

Steps:
  derivation-a    Print Node A qcow2 image derivation
  derivation-b    Print Node B qcow2 image derivation
  image-a         Build and copy Node A qcow2 to target state
  image-b         Build and copy Node B qcow2 to target state
  launch-a        Print QEMU command for Node A
  launch-b        Print QEMU command for Node B
  help            Show this help

Environment:
  GUIX_P2P_E2E_PRIVATE_DIR     State directory, default target/guix-p2p-private-store
  GUIX_P2P_E2E_IMAGE_SIZE      Image size, default 8G
  GUIX_P2P_E2E_VM_MEMORY       QEMU memory in MB, default 2048
  GUIX_P2P_E2E_VM_CPUS         QEMU CPU count, default 2
  GUIX_P2P_E2E_SUBSTITUTE_URLS Substitute URLs, default official Guix servers
EOF
}

node_system() {
    case "$1" in
        a) printf '%s\n' "$NODE_A_SYSTEM" ;;
        b) printf '%s\n' "$NODE_B_SYSTEM" ;;
        *) echo "unknown node: $1" >&2; exit 2 ;;
    esac
}

node_name() {
    case "$1" in
        a) printf '%s\n' "node-a" ;;
        b) printf '%s\n' "node-b" ;;
        *) echo "unknown node: $1" >&2; exit 2 ;;
    esac
}

image_root() {
    printf '%s/%s-image' "$STATE_DIR" "$(node_name "$1")"
}

disk_path() {
    printf '%s/%s.qcow2' "$STATE_DIR" "$(node_name "$1")"
}

serial_log() {
    printf '%s/logs/%s-serial.log' "$STATE_DIR" "$(node_name "$1")"
}

image_derivation() {
    system_file="$(node_system "$1")"
    guix system image \
        --derivation \
        --image-type=qcow2 \
        --image-size="$IMAGE_SIZE" \
        --substitute-urls="$SUBSTITUTE_URLS" \
        "$system_file"
}

build_image() {
    node="$1"
    system_file="$(node_system "$node")"
    root="$(image_root "$node")"
    disk="$(disk_path "$node")"
    output_log="$STATE_DIR/logs/$(node_name "$node")-image-build.log"

    mkdir -p "$STATE_DIR/logs"
    if [ -e "$root" ] || [ -L "$root" ]; then
        log "removing previous image root; root=$root"
        rm -f "$root"
    fi
    run_logged "$(node_name "$node") image build" "$output_log" \
        guix system image \
        --image-type=qcow2 \
        --image-size="$IMAGE_SIZE" \
        --substitute-urls="$SUBSTITUTE_URLS" \
        --root="$root" \
        "$system_file"

    source_image="$(readlink -f "$root")"
    log "copying writable disk; source=$source_image target=$disk"
    cp -f "$source_image" "$disk"
    chmod u+w "$disk"
    printf '%s\n' "$disk"
}

qemu_command() {
    node="$1"
    disk="$(disk_path "$node")"
    serial="$(serial_log "$node")"

    if [ ! -f "$disk" ]; then
        log "disk is missing; build it first with image-$node"
        exit 1
    fi

    case "$node" in
        a)
            ssh_port=2221
            dashboard_port=3031
            p2p_port=6881
            ;;
        b)
            ssh_port=2222
            dashboard_port=3032
            p2p_port=6882
            ;;
        *)
            echo "unknown node: $node" >&2
            exit 2
            ;;
    esac

    mkdir -p "$STATE_DIR/logs"
    printf '%s\n' "qemu-system-x86_64 -m $MEMORY -smp $CPUS -enable-kvm -nographic -serial file:$serial -drive file=$disk,if=virtio,format=qcow2 -nic user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$ssh_port-:22,hostfwd=tcp:127.0.0.1:$dashboard_port-:$dashboard_port,hostfwd=tcp:127.0.0.1:$p2p_port-:$p2p_port"
}

case "${1:-help}" in
    help | -h | --help)
        usage
        ;;
    derivation-a)
        image_derivation a
        ;;
    derivation-b)
        image_derivation b
        ;;
    image-a)
        build_image a
        ;;
    image-b)
        build_image b
        ;;
    launch-a)
        qemu_command a
        ;;
    launch-b)
        qemu_command b
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
