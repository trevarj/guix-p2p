#!/bin/sh
# Build and boot the disposable Guix VM used for the real P2P smoke proof.

set -eu

PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
VM_DIR="${GUIX_P2P_E2E_VM_DIR:-$PROJECT_DIR/target/guix-p2p-vm}"
OFFICIAL_IMAGE_URL="${GUIX_P2P_E2E_BASE_IMAGE_URL:-https://ftp.gnu.org/gnu/guix/guix-system-vm-image-1.5.0.x86_64-linux.qcow2}"
OFFICIAL_IMAGE="$VM_DIR/$(basename "$OFFICIAL_IMAGE_URL")"
IMAGE_SOURCE="${GUIX_P2P_E2E_IMAGE_SOURCE:-official}"
LOCAL_IMAGE_ROOT="$VM_DIR/e2e-vm-image"
DISK="$VM_DIR/e2e-vm.qcow2"
DISK_SOURCE="$VM_DIR/e2e-vm.qcow2.source"
PAYLOAD="$VM_DIR/e2e-payload.ext4"
PAYLOAD_ROOT="$VM_DIR/e2e-payload-root"
BINARY_DIR="$PAYLOAD_ROOT/bin"
GUIX_P2P_BIN="$BINARY_DIR/guix-p2p"
GUIX_P2P_E2E_BIN="$BINARY_DIR/guix-p2p-e2e"
LIB_DIR="$PAYLOAD_ROOT/lib"
PAYLOAD_SIZE_MB="${GUIX_P2P_E2E_PAYLOAD_SIZE_MB:-128}"
IMAGE_SIZE="${GUIX_P2P_E2E_VM_SIZE:-20G}"
IMAGE_REPAIR_ATTEMPTS="${GUIX_P2P_E2E_IMAGE_REPAIR_ATTEMPTS:-20}"
MEMORY="${GUIX_P2P_E2E_VM_MEMORY:-4096}"
CPUS="${GUIX_P2P_E2E_VM_CPUS:-2}"

usage() {
    cat <<EOF
Usage: scripts/e2e-vm.sh COMMAND

Commands:
  image   Ensure the base Guix qcow2 image is available
  image-local  Build the local service-baked Guix qcow2 image
  payload Build the binary payload disk
  boot    Boot a writable copy of the image under QEMU
  run     Ensure the image, then boot it
  clean   Remove generated VM state under target/guix-p2p-vm

This is the strict real-Guix proof path. By default it fetches the official
Guix VM image and attaches a small payload disk with the guix-p2p binaries.
For a fast dashboard demo, run scripts/e2e-fast-demo.sh.

Environment:
  GUIX_P2P_E2E_BASE_IMAGE_URL  Guix qcow2 URL
  GUIX_P2P_E2E_IMAGE_SOURCE    official or local, default official
  GUIX_P2P_E2E_VM_SIZE    Image size, default 20G
  GUIX_P2P_E2E_VM_MEMORY  QEMU memory in MB, default 4096
  GUIX_P2P_E2E_VM_CPUS    QEMU CPU count, default 2
  GUIX_P2P_E2E_PAYLOAD_SIZE_MB  Payload disk size, default 128
  GUIX_P2P_E2E_IMAGE_REPAIR_ATTEMPTS  Missing store path retries, default 20
EOF
}

need_runtime_tools() {
    if command -v qemu-system-x86_64 >/dev/null 2>&1 &&
        command -v qemu-img >/dev/null 2>&1 &&
        command -v mkfs.ext4 >/dev/null 2>&1 &&
        command -v patchelf >/dev/null 2>&1 &&
        { command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1; }; then
        return 0
    fi
    if [ "${GUIX_P2P_E2E_VM_TOOLS_READY:-0}" = 1 ]; then
        echo "qemu-system-x86_64, qemu-img, mkfs.ext4, patchelf, and curl or wget are required on PATH" >&2
        exit 1
    fi
    GUIX_P2P_E2E_VM_TOOLS_READY=1 exec guix shell qemu e2fsprogs patchelf curl -- "$0" "$@"
}

need_payload_tools() {
    if command -v mkfs.ext4 >/dev/null 2>&1 &&
        command -v patchelf >/dev/null 2>&1; then
        return 0
    fi
    if [ "${GUIX_P2P_E2E_PAYLOAD_TOOLS_READY:-0}" = 1 ]; then
        echo "mkfs.ext4 and patchelf are required on PATH" >&2
        exit 1
    fi
    GUIX_P2P_E2E_PAYLOAD_TOOLS_READY=1 exec guix shell e2fsprogs patchelf -- "$0" "$@"
}

build_binaries() {
    rm -rf "$PAYLOAD_ROOT"
    mkdir -p "$BINARY_DIR" "$LIB_DIR"
    if command -v cargo >/dev/null 2>&1; then
        cargo build --release -p guix-p2p -p guix-p2p-e2e
    else
        guix shell -m "$PROJECT_DIR/manifest.scm" -- \
            cargo build --release -p guix-p2p -p guix-p2p-e2e
    fi
    cp "$PROJECT_DIR/target/release/guix-p2p" "$GUIX_P2P_BIN.tmp"
    cp "$PROJECT_DIR/target/release/guix-p2p-e2e" "$GUIX_P2P_E2E_BIN.tmp"
    mv "$GUIX_P2P_BIN.tmp" "$GUIX_P2P_BIN"
    mv "$GUIX_P2P_E2E_BIN.tmp" "$GUIX_P2P_E2E_BIN"
    chmod 755 "$GUIX_P2P_BIN" "$GUIX_P2P_E2E_BIN"

    for binary in "$GUIX_P2P_BIN" "$GUIX_P2P_E2E_BIN"; do
        ldd "$binary" |
            sed -n 's/.*=> \(\/gnu\/store\/[^ ]*\).*/\1/p; s/^[[:space:]]*\(\/gnu\/store\/[^ ]*\).*/\1/p' |
            while IFS= read -r library; do
                if [ -f "$library" ]; then
                    rm -f "$LIB_DIR/$(basename "$library")"
                    cp "$library" "$LIB_DIR/$(basename "$library")"
                fi
            done
    done
    chmod 644 "$LIB_DIR"/* 2>/dev/null || true

    loader="$LIB_DIR/ld-linux-x86-64.so.2"
    if [ ! -f "$loader" ]; then
        echo "dynamic loader was not copied into $LIB_DIR" >&2
        exit 1
    fi
    for binary in "$GUIX_P2P_BIN" "$GUIX_P2P_E2E_BIN"; do
        patchelf \
            --set-interpreter "/mnt/guix-p2p-bin/lib/$(basename "$loader")" \
            --set-rpath "/mnt/guix-p2p-bin/lib" \
            "$binary"
    done

    cat >"$PAYLOAD_ROOT/run-e2e-service.sh" <<'EOF'
#!/run/current-system/profile/bin/sh
set -eu

PAYLOAD=/mnt/guix-p2p-bin

mkdir -p "$PAYLOAD"
if ! mountpoint -q "$PAYLOAD"; then
    mount -o ro LABEL=guix-p2p-bin "$PAYLOAD"
fi

mount -o remount,rw /gnu/store 2>/dev/null || true

export LD_LIBRARY_PATH="$PAYLOAD/lib"
export GUIX_P2P_E2E_BASE=/tmp/guix-p2p-e2e
export GUIX_P2P_E2E_NO_GUIX_SHELL=1
export GUIX_P2P_E2E_REMOVE_SEED_AFTER_NODE_A=1

exec "$PAYLOAD/bin/guix-p2p-e2e" container-smoke \
    --package "${GUIX_P2P_E2E_PACKAGE:-hello}" \
    --transport "${GUIX_P2P_E2E_TRANSPORT:-tcp}" \
    --guix-p2p-bin "$PAYLOAD/bin/guix-p2p" \
    --dashboard-bind 0.0.0.0 \
    --hold \
    --keep-temp
EOF
    chmod 755 "$PAYLOAD_ROOT/run-e2e-service.sh"
}

build_payload() {
    build_binaries
    dd if=/dev/zero of="$PAYLOAD.tmp" bs=1M count="$PAYLOAD_SIZE_MB" status=none
    mkfs.ext4 -q -L guix-p2p-bin -d "$PAYLOAD_ROOT" "$PAYLOAD.tmp"
    mv "$PAYLOAD.tmp" "$PAYLOAD"
    chmod 600 "$PAYLOAD"
}

fetch_official_image() {
    mkdir -p "$VM_DIR"
    if [ -s "$OFFICIAL_IMAGE" ]; then
        return 0
    fi

    echo "fetching Guix VM image: $OFFICIAL_IMAGE_URL" >&2
    if command -v curl >/dev/null 2>&1; then
        curl -fL -C - -o "$OFFICIAL_IMAGE.tmp" "$OFFICIAL_IMAGE_URL"
    elif command -v wget >/dev/null 2>&1; then
        wget -c -O "$OFFICIAL_IMAGE.tmp" "$OFFICIAL_IMAGE_URL"
    else
        echo "curl or wget is required to fetch $OFFICIAL_IMAGE_URL" >&2
        exit 1
    fi
    mv "$OFFICIAL_IMAGE.tmp" "$OFFICIAL_IMAGE"
    chmod 444 "$OFFICIAL_IMAGE"
}

build_local_image_once() {
    rm -f "$LOCAL_IMAGE_ROOT"
    guix system image \
        -t qcow2 \
        --image-size="$IMAGE_SIZE" \
        -r "$LOCAL_IMAGE_ROOT" \
        "$PROJECT_DIR/guix/e2e-vm.scm"
}

extract_invalid_store_path() {
    sed -n 's|.*\(/gnu/store/[^'"'"'` ]*\).*is not valid.*|\1|p' "$1" | tail -n 1
}

build_local_image() {
    mkdir -p "$VM_DIR"

    attempt=1
    while [ "$attempt" -le "$IMAGE_REPAIR_ATTEMPTS" ]; do
        log="$VM_DIR/image-build-$attempt.log"
        if build_local_image_once >"$log" 2>&1; then
            cat "$log"
            rm -f "$log"
            return 0
        fi

        cat "$log"
        missing="$(extract_invalid_store_path "$log")"
        if [ -z "$missing" ]; then
            echo "image build failed without a repairable missing store path; log: $log" >&2
            return 1
        fi

        echo "restoring missing store path before retry: $missing" >&2
        guix build "$missing"
        attempt=$((attempt + 1))
    done

    echo "image build still failed after $IMAGE_REPAIR_ATTEMPTS missing-path repair attempts" >&2
    return 1
}

ensure_image() {
    case "$IMAGE_SOURCE" in
        official)
            fetch_official_image
            ;;
        local)
            build_local_image
            ;;
        *)
            echo "unknown GUIX_P2P_E2E_IMAGE_SOURCE: $IMAGE_SOURCE" >&2
            echo "expected official or local" >&2
            exit 2
            ;;
    esac
}

image_source_path() {
    case "$IMAGE_SOURCE" in
        official)
            fetch_official_image
            readlink -f "$OFFICIAL_IMAGE"
            ;;
        local)
            if [ ! -e "$LOCAL_IMAGE_ROOT" ]; then
                build_local_image
            fi
            readlink -f "$LOCAL_IMAGE_ROOT"
            ;;
        *)
            echo "unknown GUIX_P2P_E2E_IMAGE_SOURCE: $IMAGE_SOURCE" >&2
            exit 2
            ;;
    esac
}

prepare_disk() {
    image_source="$(image_source_path)"
    disk_source=""
    if [ -e "$DISK_SOURCE" ]; then
        disk_source="$(cat "$DISK_SOURCE")"
    fi

    if [ ! -e "$DISK" ] || [ "$disk_source" != "$image_source" ]; then
        if [ -e "$DISK" ]; then
            echo "refreshing writable VM disk from updated image" >&2
        fi
        rm -f "$DISK.tmp"
        qemu-img create -f qcow2 -F qcow2 -b "$image_source" "$DISK.tmp" >/dev/null
        mv "$DISK.tmp" "$DISK"
        chmod 600 "$DISK"
        printf '%s\n' "$image_source" >"$DISK_SOURCE"
    fi
}

boot_vm() {
    need_runtime_tools boot
    prepare_disk
    build_payload

    accel=tcg
    if [ -e /dev/kvm ] && [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
        accel=kvm
    else
        echo "warning: /dev/kvm is unavailable; using slower QEMU TCG" >&2
    fi

    echo "dashboard ports forwarded:"
    echo "  node A: http://127.0.0.1:3031"
    echo "  node B: http://127.0.0.1:3032"
    echo "payload runner inside guest: /mnt/guix-p2p-bin/run-e2e-service.sh"
    echo "VM log inside guest: /var/log/guix-p2p-e2e.log"

    exec qemu-system-x86_64 \
        -m "$MEMORY" \
        -smp "$CPUS" \
        -accel "$accel" \
        -drive "file=$DISK,if=virtio,format=qcow2" \
        -drive "file=$PAYLOAD,if=virtio,format=raw,readonly=on" \
        -nic user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:3031-:3031,hostfwd=tcp:127.0.0.1:3032-:3032,hostfwd=tcp:127.0.0.1:2222-:22 \
        -display none \
        -serial stdio \
        -monitor none
}

clean_vm() {
    rm -rf "$VM_DIR"
}

case "${1:-}" in
    image)
        need_runtime_tools image
        ensure_image
        ;;
    image-local)
        IMAGE_SOURCE=local
        build_local_image
        ;;
    payload)
        need_payload_tools payload
        build_payload
        ;;
    boot)
        boot_vm
        ;;
    run)
        need_runtime_tools run
        ensure_image
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
