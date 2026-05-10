#!/bin/sh
# Build and boot the disposable Guix VM used for the real P2P smoke proof.

set -eu

PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
VM_DIR="${GUIX_P2P_E2E_VM_DIR:-$PROJECT_DIR/target/guix-p2p-vm}"
LOCAL_IMAGE_ROOT="$VM_DIR/e2e-vm-image"
DISK="$VM_DIR/e2e-vm.qcow2"
DISK_SOURCE="$VM_DIR/e2e-vm.qcow2.source"
PAYLOAD_ROOT="$VM_DIR/e2e-payload-root"
BINARY_DIR="$PAYLOAD_ROOT/bin"
GUIX_P2P_BIN="$BINARY_DIR/guix-p2p"
GUIX_P2P_E2E_BIN="$BINARY_DIR/guix-p2p-e2e"
STATIC_TARGET="x86_64-unknown-linux-musl"
IMAGE_SIZE="${GUIX_P2P_E2E_VM_SIZE:-20G}"
MEMORY="${GUIX_P2P_E2E_VM_MEMORY:-4096}"
CPUS="${GUIX_P2P_E2E_VM_CPUS:-2}"
DISPLAY_MODE="${GUIX_P2P_E2E_VM_DISPLAY:-none}"
LOG_DIR="${GUIX_P2P_E2E_LOG_DIR:-$VM_DIR/logs}"
SHELL_LOG="$LOG_DIR/e2e-vm.log"
IMAGE_LOG="$LOG_DIR/image-build.log"
PAYLOAD_LOG="$LOG_DIR/payload-build.log"
HEARTBEAT_SECS="${GUIX_P2P_E2E_HEARTBEAT_SECS:-30}"

timestamp() {
    date '+%Y-%m-%dT%H:%M:%S%z'
}

log() {
    mkdir -p "$LOG_DIR"
    line="$(timestamp) e2e-vm: $*"
    printf '%s\n' "$line" >&2
    printf '%s\n' "$line" >>"$SHELL_LOG"
}

log_tail() {
    path="$1"
    lines="${2:-80}"
    if [ -f "$path" ]; then
        log "last $lines lines from $path:"
        tail -n "$lines" "$path" >&2 || true
    else
        log "log file is missing: $path"
    fi
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

usage() {
    cat <<EOF
Usage: scripts/e2e-vm.sh COMMAND

Commands:
  image          Ensure the local Guix qcow2 image exists
  rebuild-image  Rebuild the local Guix qcow2 image
  payload        Build the static shared payload directory
  boot           Boot a writable copy of the image under QEMU
  run            Ensure the image, rebuild payload, then boot
  clean          Remove generated VM state under target/guix-p2p-vm

This is the strict real-Guix proof path. It builds a minimal Guix image with
the E2E Shepherd service and shares static guix-p2p binaries with the guest
through QEMU 9p.

Environment:
  GUIX_P2P_E2E_VM_DIR       Generated VM state directory
  GUIX_P2P_E2E_VM_SIZE      Image size, default 20G
  GUIX_P2P_E2E_VM_MEMORY    QEMU memory in MB, default 4096
  GUIX_P2P_E2E_VM_CPUS      QEMU CPU count, default 2
  GUIX_P2P_E2E_VM_DISPLAY   QEMU display backend, default none
  GUIX_P2P_E2E_LOG_DIR      Shell log directory, default target/guix-p2p-vm/logs
  GUIX_P2P_E2E_HEARTBEAT_SECS  Long command heartbeat interval, default 30
EOF
}

need_runtime_tools() {
    if command -v qemu-system-x86_64 >/dev/null 2>&1 &&
        command -v qemu-img >/dev/null 2>&1; then
        return 0
    fi
    if [ "${GUIX_P2P_E2E_VM_TOOLS_READY:-0}" = 1 ]; then
        echo "qemu-system-x86_64 and qemu-img are required on PATH" >&2
        exit 1
    fi
    GUIX_P2P_E2E_VM_TOOLS_READY=1 exec guix shell qemu -- "$0" "$@"
}

need_build_tools() {
    if command -v cargo >/dev/null 2>&1 &&
        command -v rustc >/dev/null 2>&1 &&
        command -v musl-gcc >/dev/null 2>&1; then
        return 0
    fi
    if [ "${GUIX_P2P_E2E_BUILD_TOOLS_READY:-0}" = 1 ]; then
        echo "cargo, rustc, and musl-gcc are required on PATH" >&2
        exit 1
    fi
    GUIX_P2P_E2E_BUILD_TOOLS_READY=1 exec guix shell -m "$PROJECT_DIR/manifest.scm" -- "$0" "$@"
}

ensure_rust_src() {
    sysroot="$(rustc --print sysroot)"
    if [ -d "$sysroot/lib/rustlib/src/rust/library" ]; then
        return 0
    fi
    cat >&2 <<EOF
Rust source is required for the E2E VM payload static build.

Install or expose a nightly Rust toolchain with the rust-src component, then rerun:
  scripts/e2e-vm.sh payload

This runner intentionally does not fall back to dynamic binaries because the
guest only receives the shared payload directory, not the host toolchain or
host /gnu/store runtime library paths.
EOF
    exit 1
}

ensure_static_binary() {
    binary="$1"
    if [ ! -x "$binary" ]; then
        echo "payload binary is missing or not executable: $binary" >&2
        exit 1
    fi

    if command -v ldd >/dev/null 2>&1; then
        deps="$(ldd "$binary" 2>&1 || true)"
        if printf '%s\n' "$deps" | grep -q '/gnu/store\|ld-linux\|libc\.so\|=>'; then
            echo "payload binary is dynamically linked, expected static: $binary" >&2
            printf '%s\n' "$deps" >&2
            exit 1
        fi
    fi
}

library_path_with() {
    name="$1"
    old_ifs="$IFS"
    IFS=:
    for dir in ${LIBRARY_PATH:-}; do
        if [ -f "$dir/$name" ]; then
            IFS="$old_ifs"
            printf '%s\n' "$dir"
            return 0
        fi
    done
    IFS="$old_ifs"
    return 1
}

write_payload_runner() {
    cat >"$PAYLOAD_ROOT/run-e2e-service.sh" <<'EOF'
#!/run/current-system/profile/bin/sh
set -eu

PAYLOAD=/mnt/guix-p2p-bin
GUEST_LOG=/var/log/guix-p2p-e2e-runner.log

guest_log() {
    line="$(date '+%Y-%m-%dT%H:%M:%S%z') e2e-vm-guest: $*"
    printf '%s\n' "$line" >&2
    printf '%s\n' "$line" >>"$GUEST_LOG" 2>/dev/null || true
}

mkdir -p "$PAYLOAD"
if ! mountpoint -q "$PAYLOAD"; then
    guest_log "mounting 9p payload share at $PAYLOAD"
    mount -t 9p -o trans=virtio,version=9p2000.L,ro guix-p2p-bin "$PAYLOAD"
else
    guest_log "payload share already mounted at $PAYLOAD"
fi

guest_log "remounting /gnu/store writable for disposable VM proof"
mount -o remount,rw /gnu/store 2>/dev/null || true

export GUIX_P2P_E2E_BASE=/tmp/guix-p2p-e2e
export GUIX_P2P_E2E_NO_GUIX_SHELL=1
export GUIX_P2P_E2E_REMOVE_SEED_AFTER_NODE_A=1

set -- \
    container-smoke \
    --package "${GUIX_P2P_E2E_PACKAGE:-hello}" \
    --transport "${GUIX_P2P_E2E_TRANSPORT:-tcp}" \
    --guix-p2p-bin "$PAYLOAD/bin/guix-p2p" \
    --dashboard-bind 0.0.0.0 \
    --hold \
    --keep-temp

if [ -n "${GUIX_P2P_E2E_STORE_PATH:-}" ]; then
    guest_log "starting guix-p2p-e2e container-smoke with explicit store path"
    exec "$PAYLOAD/bin/guix-p2p-e2e" "$@" --store-path "$GUIX_P2P_E2E_STORE_PATH"
fi

guest_log "starting guix-p2p-e2e container-smoke"
exec "$PAYLOAD/bin/guix-p2p-e2e" "$@"
EOF
    chmod 755 "$PAYLOAD_ROOT/run-e2e-service.sh"
}

build_payload() {
    log "building static payload; target=$STATIC_TARGET payload=$PAYLOAD_ROOT"
    need_build_tools payload
    ensure_rust_src

    rm -rf "$PAYLOAD_ROOT"
    mkdir -p "$BINARY_DIR"

    CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER:-rust-lld}"
    CC_x86_64_unknown_linux_musl="${CC_x86_64_unknown_linux_musl:-musl-gcc}"
    CARGO_PROFILE_RELEASE_PANIC="${CARGO_PROFILE_RELEASE_PANIC:-abort}"
    musl_lib_dir="$(dirname "$(musl-gcc -print-file-name=rcrt1.o)")"
    gcc_crt_dir="$(dirname "$(gcc -print-file-name=crtbeginS.o)")"
    unwind_lib_dir="$(library_path_with libunwind.a || true)"
    if [ -z "$unwind_lib_dir" ]; then
        echo "libunwind.a is required in LIBRARY_PATH for static musl linking" >&2
        exit 1
    fi
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS:--L native=$musl_lib_dir -L native=$gcc_crt_dir -L native=$unwind_lib_dir -C link-arg=-lm -C link-arg=-lunwind -C link-arg=-lgcc_eh}"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS
    export CC_x86_64_unknown_linux_musl
    export CARGO_PROFILE_RELEASE_PANIC

    run_with_heartbeat "payload cargo build" "$PAYLOAD_LOG" cargo build \
        -Z build-std=std,panic_abort \
        --release \
        --target "$STATIC_TARGET" \
        -p guix-p2p \
        -p guix-p2p-e2e

    cp "$PROJECT_DIR/target/$STATIC_TARGET/release/guix-p2p" "$GUIX_P2P_BIN.tmp"
    cp "$PROJECT_DIR/target/$STATIC_TARGET/release/guix-p2p-e2e" "$GUIX_P2P_E2E_BIN.tmp"
    mv "$GUIX_P2P_BIN.tmp" "$GUIX_P2P_BIN"
    mv "$GUIX_P2P_E2E_BIN.tmp" "$GUIX_P2P_E2E_BIN"
    chmod 755 "$GUIX_P2P_BIN" "$GUIX_P2P_E2E_BIN"

    ensure_static_binary "$GUIX_P2P_BIN"
    ensure_static_binary "$GUIX_P2P_E2E_BIN"
    write_payload_runner
    log "payload ready; root=$PAYLOAD_ROOT log=$PAYLOAD_LOG"
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

rebuild_image() {
    mkdir -p "$VM_DIR"
    log "rebuilding local Guix qcow2 image; size=$IMAGE_SIZE output=$LOCAL_IMAGE_ROOT"

    if run_with_heartbeat "guix system image" "$IMAGE_LOG" build_local_image_once; then
        log "image ready; output=$LOCAL_IMAGE_ROOT"
        return 0
    fi

    log_tail "$IMAGE_LOG" 100
    missing="$(extract_invalid_store_path "$IMAGE_LOG")"
    if [ -n "$missing" ]; then
        log "invalid host Guix store path detected: $missing"
        cat >&2 <<EOF
The local Guix image build failed because a host store path is marked invalid.
Repair the host Guix store manually before rerunning the E2E VM.

Exact-path repair:
  sudo guix build --repair $missing

Broader store verification and repair:
  sudo guix gc --verify=contents,repair

Then rerun:
  scripts/e2e-vm.sh rebuild-image
EOF
    else
        log "image build failed; full log: $IMAGE_LOG"
    fi
    return 1
}

ensure_image() {
    if [ -e "$LOCAL_IMAGE_ROOT" ]; then
        log "using existing image; output=$LOCAL_IMAGE_ROOT"
        return 0
    fi
    rebuild_image
}

image_source_path() {
    ensure_image
    readlink -f "$LOCAL_IMAGE_ROOT"
}

prepare_disk() {
    log "preparing writable VM disk; disk=$DISK"
    image_source="$(image_source_path)"
    disk_source=""
    if [ -e "$DISK_SOURCE" ]; then
        disk_source="$(cat "$DISK_SOURCE")"
    fi

    if [ ! -e "$DISK" ] || [ "$disk_source" != "$image_source" ]; then
        if [ -e "$DISK" ]; then
            log "refreshing writable VM disk from updated image"
        fi
        rm -f "$DISK.tmp"
        qemu-img create -f qcow2 -F qcow2 -b "$image_source" "$DISK.tmp" >/dev/null
        mv "$DISK.tmp" "$DISK"
        chmod 600 "$DISK"
        printf '%s\n' "$image_source" >"$DISK_SOURCE"
        log "writable VM disk ready; backing=$image_source"
    else
        log "using existing writable VM disk; backing=$image_source"
    fi
}

boot_vm() {
    log "boot requested; vm_dir=$VM_DIR memory=${MEMORY}M cpus=$CPUS display=$DISPLAY_MODE"
    need_runtime_tools boot
    prepare_disk
    build_payload

    accel=tcg
    if [ -e /dev/kvm ] && [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
        accel=kvm
    else
        log "warning: /dev/kvm is unavailable; using slower QEMU TCG"
    fi

    log "starting QEMU; accel=$accel disk=$DISK payload=$PAYLOAD_ROOT"
    echo "dashboard ports forwarded:"
    echo "  node A: http://127.0.0.1:3031"
    echo "  node B: http://127.0.0.1:3032"
    echo "payload runner inside guest: /mnt/guix-p2p-bin/run-e2e-service.sh"
    echo "guest service: guix-p2p-e2e Shepherd service"
    echo "display backend: $DISPLAY_MODE"
    echo "VM log inside guest: /var/log/guix-p2p-e2e.log"
    echo "guest runner log: /var/log/guix-p2p-e2e-runner.log"
    echo "host shell log: $SHELL_LOG"

    exec qemu-system-x86_64 \
        -m "$MEMORY" \
        -smp "$CPUS" \
        -accel "$accel" \
        -drive "file=$DISK,if=virtio,format=qcow2" \
        -virtfs "local,path=$PAYLOAD_ROOT,mount_tag=guix-p2p-bin,security_model=none,readonly=on" \
        -nic user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:3031-:3031,hostfwd=tcp:127.0.0.1:3032-:3032 \
        -display "$DISPLAY_MODE" \
        -serial stdio \
        -monitor none
}

clean_vm() {
    log "removing generated VM state under $VM_DIR"
    rm -rf "$VM_DIR"
}

case "${1:-}" in
    image)
        log "command=image"
        need_runtime_tools image
        ensure_image
        ;;
    rebuild-image)
        log "command=rebuild-image"
        need_runtime_tools rebuild-image
        rebuild_image
        ;;
    payload)
        log "command=payload"
        build_payload
        ;;
    boot)
        log "command=boot"
        boot_vm
        ;;
    run)
        log "command=run"
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
