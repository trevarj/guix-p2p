#!/bin/sh
# Multi-node guix-p2p substitute smoke test using Guix containers.
#
# This script is intentionally host-local: it builds the current checkout,
# starts two guix-p2p daemons in `guix shell -CN` containers, points a raw
# guix-daemon at a generated wrapper, and runs `guix build hello` through it.

set -eu

PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
BASE="${GUIX_P2P_E2E_BASE:-/tmp/guix-p2p-e2e}"
PACKAGE="${GUIX_P2P_E2E_PACKAGE:-hello}"
TRANSPORT="${GUIX_P2P_E2E_TRANSPORT:-tcp}"

NODE_A_PORT="${GUIX_P2P_E2E_NODE_A_PORT:-6881}"
NODE_B_PORT="${GUIX_P2P_E2E_NODE_B_PORT:-6882}"
NODE_A_DASH="${GUIX_P2P_E2E_NODE_A_DASH:-3031}"
NODE_B_DASH="${GUIX_P2P_E2E_NODE_B_DASH:-3032}"

PIDS=""

need() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "missing required command: $1" >&2
        exit 1
    }
}

cleanup() {
    for pid in $PIDS; do
        kill "$pid" 2>/dev/null || true
    done
}
trap cleanup EXIT INT TERM

listen_addr() {
    port="$1"
    case "$TRANSPORT" in
        tcp) printf '/ip4/127.0.0.1/tcp/%s' "$port" ;;
        quic) printf '/ip4/127.0.0.1/udp/%s/quic-v1' "$port" ;;
        *) echo "unsupported GUIX_P2P_E2E_TRANSPORT: $TRANSPORT" >&2; exit 1 ;;
    esac
}

json_field() {
    field="$1"
    sed -n "s/.*\"$field\":\"\([^\"]*\)\".*/\1/p"
}

wait_dashboard() {
    port="$1"
    label="$2"
    for _ in $(seq 1 60); do
        if curl -fsS "http://127.0.0.1:$port/api/status" >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    echo "timed out waiting for $label dashboard on port $port" >&2
    exit 1
}

raw_guix_daemon() {
    for candidate in /gnu/store/*-guix-*/bin/guix-daemon; do
        [ -x "$candidate" ] || continue
        if head -c 4 "$candidate" 2>/dev/null | od -An -tx1 | grep -q '7f 45 4c 46'; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    echo "could not find raw ELF guix-daemon under /gnu/store" >&2
    exit 1
}

make_wrapper() {
    wrapper="$1"
    socket="$2"
    p2p_bin="$3"
    real_guix="$4"

    cat > "$wrapper" <<EOF
#!/bin/sh
SOCKET="$socket"
GUIX_P2P="$p2p_bin"
REAL_GUIX="$real_guix"

case "\${1-}" in
    substitute)
        shift
        case "\${1-}" in
            --query|--substitute)
                if [ -S "\$SOCKET" ]; then
                    exec "\$GUIX_P2P" "\$@" --socket "\$SOCKET"
                fi
                exec "\$REAL_GUIX" substitute "\$@"
                ;;
            *)
                exec "\$REAL_GUIX" substitute "\$@"
                ;;
        esac
        ;;
    *)
        exec "\$REAL_GUIX" "\$@"
        ;;
esac
EOF
    chmod +x "$wrapper"
}

need cargo
need guix
need curl
need sed
need od
need grep

cd "$PROJECT_DIR"
cargo build --release

GUIX_P2P_BIN="$PROJECT_DIR/target/release/guix-p2p"
REAL_GUIX_BIN="$(readlink -f "$(command -v guix)")"
RAW_DAEMON="$(raw_guix_daemon)"
SEED_PATH="$(guix build "$PACKAGE" | tail -n 1)"

rm -rf "$BASE"
mkdir -p "$BASE/node-a" "$BASE/node-b"

NODE_A_ADDR="$(listen_addr "$NODE_A_PORT")"
NODE_B_ADDR="$(listen_addr "$NODE_B_PORT")"
NODE_A_SOCKET="$BASE/node-a.sock"
NODE_B_SOCKET="$BASE/node-b.sock"

echo "starting node A on $NODE_A_ADDR"
guix shell -CN \
    --share="$PROJECT_DIR" \
    --share="$BASE/node-a" \
    --expose=/gnu/store \
    --expose=/etc/guix \
    -- "$GUIX_P2P_BIN" --daemon \
        --listen-addr "$NODE_A_ADDR" \
        --cache-dir "$BASE/node-a" \
        --socket "$NODE_A_SOCKET" \
        --dashboard --dashboard-port "$NODE_A_DASH" \
        --policy p2p-only \
        --seed "$SEED_PATH" &
PIDS="$PIDS $!"

wait_dashboard "$NODE_A_DASH" "node A"
NODE_A_PEERID="$(curl -fsS "http://127.0.0.1:$NODE_A_DASH/api/status" | json_field peer_id)"
[ -n "$NODE_A_PEERID" ] || {
    echo "could not read node A PeerId" >&2
    exit 1
}

mkdir -p \
    "$BASE/node-b/state/db" \
    "$BASE/node-b/state/daemon-socket" \
    "$BASE/node-b/state/gcroots" \
    "$BASE/node-b/state/profiles" \
    "$BASE/node-b/state/substitute" \
    "$BASE/node-b/state/temproots" \
    "$BASE/node-b/state/userpool" \
    "$BASE/node-b/etc"
cp /etc/guix/acl "$BASE/node-b/etc/acl" 2>/dev/null || true

WRAPPER="$BASE/node-b/guix-wrapper.sh"
make_wrapper "$WRAPPER" "$NODE_B_SOCKET" "$GUIX_P2P_BIN" "$REAL_GUIX_BIN"

BOOTSTRAP="$NODE_A_ADDR/p2p/$NODE_A_PEERID"

echo "starting node B on $NODE_B_ADDR"
guix shell -CN \
    --share="$PROJECT_DIR" \
    --share="$BASE/node-b" \
    --expose=/gnu/store \
    --expose=/etc/guix \
    -- "$GUIX_P2P_BIN" --daemon \
        --listen-addr "$NODE_B_ADDR" \
        --cache-dir "$BASE/node-b" \
        --socket "$NODE_B_SOCKET" \
        --dashboard --dashboard-port "$NODE_B_DASH" \
        --policy p2p-only \
        --bootstrap-peers "$BOOTSTRAP" &
PIDS="$PIDS $!"

wait_dashboard "$NODE_B_DASH" "node B"

echo "starting isolated guix-daemon"
GUIX="$WRAPPER" \
GUIX_STATE_DIRECTORY="$BASE/node-b/state" \
GUIX_CONFIGURATION_DIRECTORY="$BASE/node-b/etc" \
"$RAW_DAEMON" \
    --disable-chroot \
    --max-jobs=0 \
    --listen="$BASE/node-b/daemon.sock" &
PIDS="$PIDS $!"

sleep 2

echo "building $PACKAGE through node B"
GUIX_DAEMON_SOCKET="$BASE/node-b/daemon.sock" guix build "$PACKAGE"

echo "node B catalog:"
curl -fsS "http://127.0.0.1:$NODE_B_DASH/api/catalog"
printf '\n'

echo "node B seeds:"
curl -fsS "http://127.0.0.1:$NODE_B_DASH/api/seeds"
printf '\n'

echo "E2E container test completed"
