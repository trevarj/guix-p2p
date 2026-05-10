#!/bin/sh
# Build and launch the private-store VM proof scaffolding.

set -eu

PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
STATE_DIR="${GUIX_P2P_E2E_PRIVATE_DIR:-$PROJECT_DIR/target/guix-p2p-private-store}"
IMAGE_SIZE="${GUIX_P2P_E2E_IMAGE_SIZE:-8G}"
MEMORY="${GUIX_P2P_E2E_VM_MEMORY:-2048}"
CPUS="${GUIX_P2P_E2E_VM_CPUS:-2}"
SUBSTITUTE_URLS="${GUIX_P2P_E2E_SUBSTITUTE_URLS:-https://ci.guix.gnu.org https://bordeaux.guix.gnu.org}"
NODE_SYSTEM="$PROJECT_DIR/guix/e2e-private-node.scm"
GUIX_P2P_BINARY="${GUIX_P2P_E2E_BINARY:-$PROJECT_DIR/target/release/guix-p2p}"

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
  derivation      Print the base qcow2 image derivation
  image           Build the base qcow2 and copy it to Node A and Node B disks
  launch-a        Print QEMU command for Node A
  launch-b        Print QEMU command for Node B
  run-a           Run Node A under QEMU in the foreground
  run-b           Run Node B under QEMU in the foreground
  help            Show this help

Environment:
  GUIX_P2P_E2E_PRIVATE_DIR     State directory, default target/guix-p2p-private-store
  GUIX_P2P_E2E_IMAGE_SIZE      Image size, default 8G
  GUIX_P2P_E2E_VM_MEMORY       QEMU memory in MB, default 2048
  GUIX_P2P_E2E_VM_CPUS         QEMU CPU count, default 2
  GUIX_P2P_E2E_SUBSTITUTE_URLS Substitute URLs, default official Guix servers
  GUIX_P2P_E2E_BINARY          guix-p2p binary embedded in the image,
                               default target/release/guix-p2p
EOF
}

node_name() {
    case "$1" in
        a) printf '%s\n' "node-a" ;;
        b) printf '%s\n' "node-b" ;;
        *) echo "unknown node: $1" >&2; exit 2 ;;
    esac
}

base_image_root() {
    printf '%s/base-image' "$STATE_DIR"
}

base_disk_path() {
    printf '%s/base.qcow2' "$STATE_DIR"
}

disk_path() {
    printf '%s/%s.qcow2' "$STATE_DIR" "$(node_name "$1")"
}

serial_log() {
    printf '%s/logs/%s-serial.log' "$STATE_DIR" "$(node_name "$1")"
}

ensure_guix_p2p_binary() {
    if [ ! -x "$GUIX_P2P_BINARY" ]; then
        log "guix-p2p binary is missing or not executable; build it first with: cargo build --release"
        log "expected binary: $GUIX_P2P_BINARY"
        exit 1
    fi
}

image_derivation() {
    ensure_guix_p2p_binary
    GUIX_P2P_E2E_BINARY="$GUIX_P2P_BINARY" guix system image \
        --derivation \
        --image-type=qcow2 \
        --image-size="$IMAGE_SIZE" \
        --substitute-urls="$SUBSTITUTE_URLS" \
        "$NODE_SYSTEM"
}

build_image() {
    ensure_guix_p2p_binary
    root="$(base_image_root)"
    base_disk="$(base_disk_path)"
    output_log="$STATE_DIR/logs/base-image-build.log"

    mkdir -p "$STATE_DIR/logs"
    if [ -e "$root" ] || [ -L "$root" ]; then
        log "removing previous image root; root=$root"
        rm -f "$root"
    fi
    run_logged "base image build" "$output_log" \
        env GUIX_P2P_E2E_BINARY="$GUIX_P2P_BINARY" guix system image \
        --image-type=qcow2 \
        --image-size="$IMAGE_SIZE" \
        --substitute-urls="$SUBSTITUTE_URLS" \
        --root="$root" \
        "$NODE_SYSTEM"

    source_image="$(readlink -f "$root")"
    log "copying base disk; source=$source_image target=$base_disk"
    cp -f "$source_image" "$base_disk"
    chmod u+w "$base_disk"

    for node in a b; do
        disk="$(disk_path "$node")"
        log "copying writable $(node_name "$node") disk; source=$base_disk target=$disk"
        cp -f "$base_disk" "$disk"
        chmod u+w "$disk"
        printf '%s\n' "$disk"
    done
}

qemu_ports() {
    node="$1"

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
}

qemu_command() {
    node="$1"
    disk="$(disk_path "$node")"
    serial="$(serial_log "$node")"

    if [ ! -f "$disk" ]; then
        log "disk is missing; build it first with image"
        exit 1
    fi

    qemu_ports "$node"

    mkdir -p "$STATE_DIR/logs"
    printf '%s\n' "qemu-system-x86_64 -m $MEMORY -smp $CPUS -enable-kvm -nographic -serial file:$serial -drive file=$disk,if=virtio,format=qcow2 -nic user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$ssh_port-:22,hostfwd=tcp:127.0.0.1:$dashboard_port-:$dashboard_port,hostfwd=tcp:127.0.0.1:$p2p_port-:$p2p_port"
}

run_qemu() {
    node="$1"
    disk="$(disk_path "$node")"
    serial="$(serial_log "$node")"

    if [ ! -f "$disk" ]; then
        log "disk is missing; build it first with image"
        exit 1
    fi

    qemu_ports "$node"
    mkdir -p "$STATE_DIR/logs"
    : >"$serial"
    log "running $(node_name "$node"); serial=$serial"
    log "watch serial output with: tail -f $serial"

    if command -v qemu-system-x86_64 >/dev/null 2>&1; then
        exec qemu-system-x86_64 \
            -m "$MEMORY" \
            -smp "$CPUS" \
            -enable-kvm \
            -nographic \
            -serial "file:$serial" \
            -drive "file=$disk,if=virtio,format=qcow2" \
            -nic "user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$ssh_port-:22,hostfwd=tcp:127.0.0.1:$dashboard_port-:$dashboard_port,hostfwd=tcp:127.0.0.1:$p2p_port-:$p2p_port"
    fi

    exec guix shell qemu -- qemu-system-x86_64 \
        -m "$MEMORY" \
        -smp "$CPUS" \
        -enable-kvm \
        -nographic \
        -serial "file:$serial" \
        -drive "file=$disk,if=virtio,format=qcow2" \
        -nic "user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$ssh_port-:22,hostfwd=tcp:127.0.0.1:$dashboard_port-:$dashboard_port,hostfwd=tcp:127.0.0.1:$p2p_port-:$p2p_port"
}

case "${1:-help}" in
    help | -h | --help)
        usage
        ;;
    derivation)
        image_derivation
        ;;
    derivation-a | derivation-b)
        log "$1 is deprecated; use derivation"
        image_derivation
        ;;
    image)
        build_image
        ;;
    image-a | image-b)
        log "$1 is deprecated; use image"
        build_image
        ;;
    launch-a)
        qemu_command a
        ;;
    launch-b)
        qemu_command b
        ;;
    run-a)
        run_qemu a
        ;;
    run-b)
        run_qemu b
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
