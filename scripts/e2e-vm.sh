#!/bin/sh
# Build and boot the disposable Guix VM used for the real P2P smoke proof.

set -eu

PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
VM_DIR="${GUIX_P2P_E2E_VM_DIR:-$PROJECT_DIR/target/guix-p2p-vm}"
IMAGE_ROOT="$VM_DIR/e2e-vm-image"
DISK="$VM_DIR/e2e-vm.qcow2"
IMAGE_SIZE="${GUIX_P2P_E2E_VM_SIZE:-20G}"
MEMORY="${GUIX_P2P_E2E_VM_MEMORY:-4096}"
CPUS="${GUIX_P2P_E2E_VM_CPUS:-2}"

usage() {
    cat <<EOF
Usage: scripts/e2e-vm.sh COMMAND

Commands:
  image   Build the disposable Guix qcow2 image
  boot    Boot a writable copy of the image under QEMU
  run     Build the image if needed, then boot it
  clean   Remove generated VM state under target/guix-p2p-vm

Environment:
  GUIX_P2P_E2E_VM_SIZE    Image size, default 20G
  GUIX_P2P_E2E_VM_MEMORY  QEMU memory in MB, default 4096
  GUIX_P2P_E2E_VM_CPUS    QEMU CPU count, default 2
EOF
}

need_qemu() {
    if command -v qemu-system-x86_64 >/dev/null 2>&1; then
        return 0
    fi
    if [ "${GUIX_P2P_E2E_VM_QEMU_READY:-0}" = 1 ]; then
        echo "qemu-system-x86_64 is not available on PATH" >&2
        exit 1
    fi
    GUIX_P2P_E2E_VM_QEMU_READY=1 exec guix shell qemu -- "$0" "$@"
}

build_image() {
    mkdir -p "$VM_DIR"
    guix system image \
        -t qcow2 \
        --image-size="$IMAGE_SIZE" \
        -r "$IMAGE_ROOT" \
        "$PROJECT_DIR/guix/e2e-vm.scm"
}

prepare_disk() {
    if [ ! -e "$IMAGE_ROOT" ]; then
        build_image
    fi
    if [ ! -e "$DISK" ]; then
        cp "$(readlink -f "$IMAGE_ROOT")" "$DISK"
        chmod 600 "$DISK"
    fi
}

boot_vm() {
    need_qemu boot
    prepare_disk

    accel=tcg
    if [ -e /dev/kvm ] && [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
        accel=kvm
    else
        echo "warning: /dev/kvm is unavailable; using slower QEMU TCG" >&2
    fi

    echo "dashboard ports forwarded:"
    echo "  node A: http://127.0.0.1:3031"
    echo "  node B: http://127.0.0.1:3032"
    echo "VM log inside guest: /var/log/guix-p2p-e2e.log"

    exec qemu-system-x86_64 \
        -m "$MEMORY" \
        -smp "$CPUS" \
        -accel "$accel" \
        -drive "file=$DISK,if=virtio,format=qcow2" \
        -nic user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:3031-:3031,hostfwd=tcp:127.0.0.1:3032-:3032,hostfwd=tcp:127.0.0.1:2222-:22 \
        -virtfs "local,path=$PROJECT_DIR,mount_tag=guix_p2p,security_model=mapped-xattr,id=guix_p2p" \
        -nographic \
        -serial mon:stdio
}

clean_vm() {
    rm -rf "$VM_DIR"
}

case "${1:-}" in
    image)
        build_image
        ;;
    boot)
        boot_vm
        ;;
    run)
        build_image
        boot_vm
        ;;
    clean)
        clean_vm
        ;;
    *)
        usage
        exit 2
        ;;
esac
