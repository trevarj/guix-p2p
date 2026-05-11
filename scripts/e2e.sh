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
SSH_DIR="$STATE_DIR/ssh"
SSH_HOST_KEY="${GUIX_P2P_E2E_SSH_HOST_KEY:-$SSH_DIR/ssh_host_ed25519_key}"
SSH_HOST_KEY_PUB="${GUIX_P2P_E2E_SSH_HOST_KEY_PUB:-$SSH_HOST_KEY.pub}"
SSH_CLIENT_KEY="${GUIX_P2P_E2E_SSH_CLIENT_KEY:-$SSH_DIR/e2e_ed25519}"
SSH_CLIENT_KEY_PUB="${GUIX_P2P_E2E_SSH_AUTHORIZED_KEY:-$SSH_CLIENT_KEY.pub}"

timestamp() {
    date '+%Y-%m-%dT%H:%M:%S%z'
}

log() {
    mkdir -p "$STATE_DIR/logs"
    printf '%s e2e: %s\n' "$(timestamp)" "$*" >&2
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
Usage: scripts/e2e.sh STEP

Steps:
  derivation      Print the base qcow2 image derivation
  image           Build the base qcow2 and copy it to Node A and Node B disks
  reset-disks     Copy base.qcow2 to fresh Node A and Node B disks
  run-a           Run Node A under QEMU in the foreground
  run-b           Run Node B under QEMU in the foreground
  ssh-a           SSH to Node A with the persistent test key
  ssh-b           SSH to Node B with the persistent test key
  wait-ssh        Wait until SSH accepts connections on both nodes
  push-binary     Copy target/release/guix-p2p to /tmp/guix-p2p on both nodes
  push-binary-a   Copy target/release/guix-p2p to /tmp/guix-p2p on Node A
  push-binary-b   Copy target/release/guix-p2p to /tmp/guix-p2p on Node B
  node-a [PKG]    Start Node A helper, seed PKG, and save STORE_PATH/PEER_ID
  env             Print saved STORE_PATH/PEER_ID export lines
  node-b [PATH PEER]
                  Start Node B helper for PATH, bootstrapped to PEER
  prewarm-b [PATH] [PKG]
                  Build PKG normally on Node B, then delete PATH
  daemon-b        Start temporary Node B guix-daemon through guix-p2p wrapper
  prove-b [PATH] [PKG]
                  Run guix build PKG through temporary Node B daemon
  logs-a          Tail Node A guix-p2p log
  logs-b          Tail Node B guix-p2p log
  daemon-log-b    Tail temporary Node B guix-daemon log
  help            Show this help

Environment:
  GUIX_P2P_E2E_PRIVATE_DIR     State directory, default target/guix-p2p-private-store
  GUIX_P2P_E2E_IMAGE_SIZE      Image size, default 8G
  GUIX_P2P_E2E_VM_MEMORY       QEMU memory in MB, default 2048
  GUIX_P2P_E2E_VM_CPUS         QEMU CPU count, default 2
  GUIX_P2P_E2E_ENABLE_KVM      auto, true, or false; default auto
  GUIX_P2P_E2E_A_SSH_PORT      Node A host SSH port, default 2221
  GUIX_P2P_E2E_B_SSH_PORT      Node B host SSH port, default 2222
  GUIX_P2P_E2E_A_DASHBOARD_PORT Node A host/dashboard port, default 3031
  GUIX_P2P_E2E_B_DASHBOARD_PORT Node B host/dashboard port, default 3032
  GUIX_P2P_E2E_FORWARD_DASHBOARD Forward dashboard ports, default true
  GUIX_P2P_E2E_A_P2P_PORT      Node A host P2P port, default 6881
  GUIX_P2P_E2E_B_P2P_PORT      Node B host P2P port, default 6882
  GUIX_P2P_E2E_SUBSTITUTE_URLS Substitute URLs, default official Guix servers
  GUIX_P2P_E2E_BINARY          guix-p2p binary embedded in the image,
                               default target/release/guix-p2p
  GUIX_P2P_E2E_SSH_HOST_KEY    Persistent test SSH host key embedded in image,
                               default target/guix-p2p-private-store/ssh/ssh_host_ed25519_key
  GUIX_P2P_E2E_SSH_CLIENT_KEY  Persistent test SSH client key authorized for e2e,
                               default target/guix-p2p-private-store/ssh/e2e_ed25519
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

proof_env_path() {
    printf '%s/e2e.env' "$STATE_DIR"
}

ensure_guix_p2p_binary() {
    if [ ! -x "$GUIX_P2P_BINARY" ]; then
        log "guix-p2p binary is missing or not executable; build it first with: cargo build --release"
        log "expected binary: $GUIX_P2P_BINARY"
        exit 1
    fi
}

ensure_ssh_host_key() {
    if [ -f "$SSH_HOST_KEY" ] && [ -f "$SSH_HOST_KEY_PUB" ]; then
        return
    fi

    mkdir -p "$SSH_DIR"
    log "generating persistent test SSH host key; key=$SSH_HOST_KEY"
    if command -v ssh-keygen >/dev/null 2>&1; then
        ssh-keygen -t ed25519 -N "" -C "guix-p2p-e2e-private-store" -f "$SSH_HOST_KEY" >/dev/null
    else
        guix shell openssh -- ssh-keygen -t ed25519 -N "" -C "guix-p2p-e2e-private-store" -f "$SSH_HOST_KEY" >/dev/null
    fi
    chmod 600 "$SSH_HOST_KEY"
    chmod 644 "$SSH_HOST_KEY_PUB"
}

ensure_ssh_client_key() {
    if [ -f "$SSH_CLIENT_KEY" ] && [ -f "$SSH_CLIENT_KEY_PUB" ]; then
        return
    fi

    mkdir -p "$SSH_DIR"
    log "generating persistent test SSH client key; key=$SSH_CLIENT_KEY"
    if command -v ssh-keygen >/dev/null 2>&1; then
        ssh-keygen -t ed25519 -N "" -C "guix-p2p-e2e-client" -f "$SSH_CLIENT_KEY" >/dev/null
    else
        guix shell openssh -- ssh-keygen -t ed25519 -N "" -C "guix-p2p-e2e-client" -f "$SSH_CLIENT_KEY" >/dev/null
    fi
    chmod 600 "$SSH_CLIENT_KEY"
    chmod 644 "$SSH_CLIENT_KEY_PUB"
}

image_derivation() {
    ensure_guix_p2p_binary
    ensure_ssh_host_key
    ensure_ssh_client_key
    GUIX_P2P_E2E_BINARY="$GUIX_P2P_BINARY" \
        GUIX_P2P_E2E_SSH_HOST_KEY="$SSH_HOST_KEY" \
        GUIX_P2P_E2E_SSH_HOST_KEY_PUB="$SSH_HOST_KEY_PUB" \
        GUIX_P2P_E2E_SSH_AUTHORIZED_KEY="$SSH_CLIENT_KEY_PUB" \
        guix system image \
        --derivation \
        --image-type=qcow2 \
        --image-size="$IMAGE_SIZE" \
        --substitute-urls="$SUBSTITUTE_URLS" \
        "$NODE_SYSTEM"
}

build_image() {
    ensure_guix_p2p_binary
    ensure_ssh_host_key
    ensure_ssh_client_key
    root="$(base_image_root)"
    base_disk="$(base_disk_path)"
    output_log="$STATE_DIR/logs/base-image-build.log"

    mkdir -p "$STATE_DIR/logs"
    if [ -e "$root" ] || [ -L "$root" ]; then
        log "removing previous image root; root=$root"
        rm -f "$root"
    fi
    run_logged "base image build" "$output_log" \
        env GUIX_P2P_E2E_BINARY="$GUIX_P2P_BINARY" \
            GUIX_P2P_E2E_SSH_HOST_KEY="$SSH_HOST_KEY" \
            GUIX_P2P_E2E_SSH_HOST_KEY_PUB="$SSH_HOST_KEY_PUB" \
            GUIX_P2P_E2E_SSH_AUTHORIZED_KEY="$SSH_CLIENT_KEY_PUB" \
            guix system image \
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

reset_disks() {
    base_disk="$(base_disk_path)"
    if [ ! -f "$base_disk" ]; then
        log "base disk is missing; build it first with: scripts/e2e.sh image"
        exit 1
    fi

    rm -f "$(proof_env_path)"
    for node in a b; do
        disk="$(disk_path "$node")"
        log "resetting writable $(node_name "$node") disk; source=$base_disk target=$disk"
        cp -f "$base_disk" "$disk"
        chmod u+w "$disk"
        printf '%s\n' "$disk"
    done
}

ssh_node() {
    node="$1"
    ensure_ssh_client_key
    qemu_ports "$node"
    exec ssh \
        -i "$SSH_CLIENT_KEY" \
        -o UserKnownHostsFile="$SSH_DIR/known_hosts" \
        -o StrictHostKeyChecking=accept-new \
        -o ConnectTimeout=10 \
        -p "$ssh_port" \
        e2e@127.0.0.1
}

quote() {
    printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

write_proof_env() {
    store_path="$1"
    peer_id="$2"
    package="$3"
    env_file="$(proof_env_path)"

    mkdir -p "$STATE_DIR"
    {
        printf 'export STORE_PATH=%s\n' "$(quote "$store_path")"
        printf 'export PEER_ID=%s\n' "$(quote "$peer_id")"
        printf 'export E2E_PACKAGE=%s\n' "$(quote "$package")"
    } >"$env_file"
}

print_proof_env() {
    env_file="$(proof_env_path)"
    if [ ! -f "$env_file" ]; then
        log "no saved E2E environment found; run: scripts/e2e.sh node-a hello"
        exit 1
    fi

    cat "$env_file"
}

load_proof_env() {
    env_file="$(proof_env_path)"
    if [ -f "$env_file" ]; then
        # shellcheck disable=SC1090
        . "$env_file"
    fi
}

resolve_store_path() {
    value="${1:-}"
    if [ -n "$value" ]; then
        printf '%s\n' "$value"
        return
    fi

    load_proof_env
    if [ -n "${STORE_PATH:-}" ]; then
        printf '%s\n' "$STORE_PATH"
        return
    fi

    log "missing STORE_PATH; pass it explicitly or run: scripts/e2e.sh node-a hello"
    exit 2
}

resolve_peer_id() {
    value="${1:-}"
    if [ -n "$value" ]; then
        printf '%s\n' "$value"
        return
    fi

    load_proof_env
    if [ -n "${PEER_ID:-}" ]; then
        printf '%s\n' "$PEER_ID"
        return
    fi

    log "missing PEER_ID; pass it explicitly or run: scripts/e2e.sh node-a hello"
    exit 2
}

ssh_run() {
    node="$1"
    shift
    ensure_ssh_client_key
    qemu_ports "$node"
    ssh \
        -i "$SSH_CLIENT_KEY" \
        -o UserKnownHostsFile="$SSH_DIR/known_hosts" \
        -o StrictHostKeyChecking=accept-new \
        -o BatchMode=yes \
        -o ConnectTimeout=10 \
        -p "$ssh_port" \
        e2e@127.0.0.1 \
        "$@"
}

wait_ssh_node() {
    node="$1"
    qemu_ports "$node"
    i=0
    while :; do
        if ssh_run "$node" "true" >/dev/null 2>&1; then
            log "$(node_name "$node") SSH ready on port $ssh_port"
            return
        fi
        i=$((i + 1))
        if [ "$i" -gt 120 ]; then
            log "$(node_name "$node") SSH did not become ready on port $ssh_port"
            exit 1
        fi
        sleep 1
    done
}

push_binary() {
    node="$1"
    ensure_guix_p2p_binary
    ensure_ssh_client_key
    libgcrypt_runtime="$(guix build libgcrypt | tail -n 1)"
    qemu_ports "$node"
    scp \
        -i "$SSH_CLIENT_KEY" \
        -o UserKnownHostsFile="$SSH_DIR/known_hosts" \
        -o StrictHostKeyChecking=accept-new \
        -o BatchMode=yes \
        -o ConnectTimeout=10 \
        -P "$ssh_port" \
        "$GUIX_P2P_BINARY" \
        e2e@127.0.0.1:/tmp/guix-p2p-real
    remote_install="
set -eu
cat > /tmp/guix-p2p <<'EOF'
#!/bin/sh
set -eu
LIBGCRYPT=\"\${GUIX_P2P_E2E_LIBGCRYPT:-$libgcrypt_runtime}\"
export LD_LIBRARY_PATH=\"\$LIBGCRYPT/lib:/run/current-system/profile/lib\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}\"
exec /tmp/guix-p2p-real \"\$@\"
EOF
chmod 755 /tmp/guix-p2p /tmp/guix-p2p-real
"
    ssh \
        -i "$SSH_CLIENT_KEY" \
        -o UserKnownHostsFile="$SSH_DIR/known_hosts" \
        -o StrictHostKeyChecking=accept-new \
        -o BatchMode=yes \
        -o ConnectTimeout=10 \
        -p "$ssh_port" \
        e2e@127.0.0.1 \
        "$remote_install"
    log "pushed binary to $(node_name "$node"):/tmp/guix-p2p"
}

node_a_helper() {
    package="${1:-hello}"
    output="$(ssh_run a "GUIX_P2P_E2E_P2P_BIN=/tmp/guix-p2p guix-p2p-e2e-node-a $(quote "$package")")"
    printf '%s\n' "$output"

    store_path="$(printf '%s\n' "$output" | sed -n 's/^store_path=//p' | tail -n 1)"
    peer_id="$(printf '%s\n' "$output" | sed -n 's/^peer_id=//p' | tail -n 1)"

    if [ -z "$store_path" ] || [ -z "$peer_id" ]; then
        log "node-a output did not include store_path and peer_id"
        exit 1
    fi

    write_proof_env "$store_path" "$peer_id" "$package"
    env_file="$(proof_env_path)"
    log "saved E2E environment; file=$env_file"
    printf '\n%s\n' "# Optional shell exports:"
    print_proof_env
    printf '%s\n' "# Or load them with: eval \"\$(scripts/e2e.sh env)\""
}

node_b_helper() {
    store_path="$(resolve_store_path "${1:-}")"
    peer_id="$(resolve_peer_id "${2:-}")"
    qemu_ports a
    node_a_p2p_port="$p2p_port"
    ssh_run b "GUIX_P2P_E2E_P2P_BIN=/tmp/guix-p2p GUIX_P2P_E2E_B_BOOTSTRAP=$(quote "/ip4/10.0.2.2/tcp/$node_a_p2p_port/p2p/$peer_id") guix-p2p-e2e-node-b $(quote "$store_path") $(quote "$peer_id")"
}

prewarm_b() {
    store_path="$(resolve_store_path "${1:-}")"
    package="${2:-hello}"
    ssh_run b "set -eu; guix build --no-grafts $(quote "$package"); guix gc -D $(quote "$store_path"); test ! -e $(quote "$store_path") && echo TARGET_ABSENT_AFTER_DELETE"
}

daemon_b() {
    remote='
set -eu
cat > /tmp/e2e-guix-wrapper <<'"'"'EOF'"'"'
#!/bin/sh
set -eu
SOCKET=/tmp/guix-p2p-b/guix-p2p.sock
GUIX_P2P=/tmp/guix-p2p
REAL_GUIX=/run/current-system/profile/bin/guix

case "${1-}" in
  substitute)
    shift
    case "${1-}" in
      --query|--substitute)
        exec "$GUIX_P2P" "$@" --socket "$SOCKET"
        ;;
      *)
        exec "$REAL_GUIX" substitute "$@"
        ;;
    esac
    ;;
  *)
    exec "$REAL_GUIX" "$@"
    ;;
esac
EOF
chmod +x /tmp/e2e-guix-wrapper
printf "e2e\n" | sudo -S sh -c '"'"'
mount -o remount,rw /gnu/store
kill $(cat /tmp/e2e-guix-daemon.pid 2>/dev/null) 2>/dev/null || true
rm -f /tmp/e2e-guix-daemon.sock /tmp/e2e-guix-daemon.log /tmp/e2e-guix-daemon.pid
GUIX=/tmp/e2e-guix-wrapper /run/current-system/profile/bin/guix-daemon \
  --disable-chroot \
  --build-users-group=guixbuild \
  --max-jobs=0 \
  --listen=/tmp/e2e-guix-daemon.sock \
  > /tmp/e2e-guix-daemon.log 2>&1 &
echo $! > /tmp/e2e-guix-daemon.pid
'"'"'
i=0
while [ ! -S /tmp/e2e-guix-daemon.sock ]; do
    i=$((i + 1))
    if [ "$i" -gt 30 ]; then
        echo DAEMON_SOCKET_TIMEOUT
        printf "e2e\n" | sudo -S cat /tmp/e2e-guix-daemon.log
        exit 1
    fi
    sleep 1
done
echo GUIX_DAEMON_SOCKET=/tmp/e2e-guix-daemon.sock
'
    ssh_run b "$remote"
}

prove_b() {
    store_path="$(resolve_store_path "${1:-}")"
    package="${2:-hello}"
    ssh_run b "set -eu; test ! -e $(quote "$store_path"); GUIX_DAEMON_SOCKET=/tmp/e2e-guix-daemon.sock guix build --no-grafts $(quote "$package"); test -d $(quote "$store_path") && echo IMPORTED_HELLO_IN_NODE_B_STORE"
}

tail_log() {
    node="$1"
    case "$node" in
        a) ssh_run a "tail -f /tmp/guix-p2p-a.log" ;;
        b) ssh_run b "tail -f /tmp/guix-p2p-b.log" ;;
        daemon-b) ssh_run b "tail -f /tmp/e2e-guix-daemon.log" ;;
        *) echo "unknown log node: $node" >&2; exit 2 ;;
    esac
}

qemu_ports() {
    node="$1"

    case "$node" in
        a)
            ssh_port="${GUIX_P2P_E2E_A_SSH_PORT:-2221}"
            dashboard_port="${GUIX_P2P_E2E_A_DASHBOARD_PORT:-3031}"
            p2p_port="${GUIX_P2P_E2E_A_P2P_PORT:-6881}"
            guest_dashboard_port=3031
            guest_p2p_port=6881
            ;;
        b)
            ssh_port="${GUIX_P2P_E2E_B_SSH_PORT:-2222}"
            dashboard_port="${GUIX_P2P_E2E_B_DASHBOARD_PORT:-3032}"
            p2p_port="${GUIX_P2P_E2E_B_P2P_PORT:-6882}"
            guest_dashboard_port=3032
            guest_p2p_port=6882
            ;;
        *)
            echo "unknown node: $node" >&2
            exit 2
            ;;
    esac
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

    kvm_arg=
    case "${GUIX_P2P_E2E_ENABLE_KVM:-auto}" in
        true | yes | 1)
            kvm_arg=-enable-kvm
            ;;
        false | no | 0)
            ;;
        auto)
            if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
                kvm_arg=-enable-kvm
            else
                log "KVM unavailable; using QEMU software emulation"
            fi
            ;;
        *)
            log "invalid GUIX_P2P_E2E_ENABLE_KVM=${GUIX_P2P_E2E_ENABLE_KVM}; expected auto, true, or false"
            exit 2
            ;;
    esac

    netdev="user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$ssh_port-:22"
    case "${GUIX_P2P_E2E_FORWARD_DASHBOARD:-true}" in
        true | yes | 1)
            netdev="$netdev,hostfwd=tcp:127.0.0.1:$dashboard_port-:$guest_dashboard_port"
            ;;
        false | no | 0)
            ;;
        *)
            log "invalid GUIX_P2P_E2E_FORWARD_DASHBOARD=${GUIX_P2P_E2E_FORWARD_DASHBOARD}; expected true or false"
            exit 2
            ;;
    esac
    netdev="$netdev,hostfwd=tcp:127.0.0.1:$p2p_port-:$guest_p2p_port"

    set -- \
        -m "$MEMORY" \
        -smp "$CPUS" \
        -nographic \
        -serial "file:$serial" \
        -drive "file=$disk,if=virtio,format=qcow2" \
        -nic "$netdev"
    if [ -n "$kvm_arg" ]; then
        set -- "$kvm_arg" "$@"
    fi

    if command -v qemu-system-x86_64 >/dev/null 2>&1; then
        exec qemu-system-x86_64 "$@"
    fi

    exec guix shell qemu -- qemu-system-x86_64 "$@"
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
    reset-disks)
        reset_disks
        ;;
    image-a | image-b)
        log "$1 is deprecated; use image"
        build_image
        ;;
    run-a)
        run_qemu a
        ;;
    run-b)
        run_qemu b
        ;;
    ssh-a)
        ssh_node a
        ;;
    ssh-b)
        ssh_node b
        ;;
    push-binary)
        push_binary a
        push_binary b
        ;;
    push-binary-a)
        push_binary a
        ;;
    push-binary-b)
        push_binary b
        ;;
    wait-ssh)
        wait_ssh_node a
        wait_ssh_node b
        ;;
    node-a)
        node_a_helper "${2:-hello}"
        ;;
    env)
        print_proof_env
        ;;
    node-b)
        node_b_helper "${2:-}" "${3:-}"
        ;;
    prewarm-b)
        prewarm_b "${2:-}" "${3:-hello}"
        ;;
    daemon-b)
        daemon_b
        ;;
    prove-b)
        prove_b "${2:-}" "${3:-hello}"
        ;;
    logs-a)
        tail_log a
        ;;
    logs-b)
        tail_log b
        ;;
    daemon-log-b)
        tail_log daemon-b
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
