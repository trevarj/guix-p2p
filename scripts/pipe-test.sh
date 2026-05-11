#!/bin/sh
# pipe-test.sh — Manual end-to-end test of the guix-p2p daemon protocol
#
# Tests the fd 4 and socket protocols without involving guix-daemon.
# Requires: guix-p2p built, socat, a running guix system
#
# Usage:
#   ./scripts/pipe-test.sh              # full test
#   ./scripts/pipe-test.sh --skip-build # skip cargo build
#   ./scripts/pipe-test.sh --cleanup    # kill daemon and clean up

set -eu

BASEDIR="$(cd "$(dirname "$0")/.." && pwd)"
BINARY="$BASEDIR/target/release/guix-p2p"
CACHE_DIR="/tmp/guix-p2p-pipe-test"
SOCKET="$CACHE_DIR/guix-p2p.sock"
DAEMON_PID=""
SKIP_BUILD=0
CLEANUP_ONLY=0

for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=1 ;;
        --cleanup) CLEANUP_ONLY=1 ;;
    esac
done

cleanup() {
    if [ -n "$DAEMON_PID" ]; then
        echo "Stopping daemon (PID $DAEMON_PID)"
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
    rm -rf "$CACHE_DIR"
    echo "Cleaned up $CACHE_DIR"
}

if [ "$CLEANUP_ONLY" = 1 ]; then
    # Try to find and kill any running test daemon
    if [ -S "$SOCKET" ]; then
        DAEMON_PID=$(ps aux | grep "guix-p2p.*$CACHE_DIR" | grep -v grep | awk '{print $2}' | head -1)
        cleanup
    else
        rm -rf "$CACHE_DIR"
        echo "Nothing to clean up"
    fi
    exit 0
fi

trap cleanup EXIT INT TERM

echo "=== guix-p2p Pipe Test ==="
echo ""

# Step 0: Build
if [ "$SKIP_BUILD" = 0 ]; then
    echo "--- Step 0: Building guix-p2p ---"
    cd "$BASEDIR"
    if command -v cargo >/dev/null 2>&1; then
        cargo build --release
    else
        guix shell -m manifest.scm -- cargo build --release
    fi
    echo ""
fi

if [ ! -x "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY"
    echo "Run: guix shell -m manifest.scm -- cargo build --release"
    exit 1
fi

# Step 1: Start daemon
echo "--- Step 1: Starting daemon ---"
rm -rf "$CACHE_DIR"
mkdir -p "$CACHE_DIR"

"$BINARY" --daemon \
    --cache-dir "$CACHE_DIR" \
    --socket "$SOCKET" \
    --dashboard --dashboard-port 3030 \
    --policy p2p-first \
    &

DAEMON_PID=$!
echo "Daemon PID: $DAEMON_PID"

# Wait for daemon to start
echo "Waiting for daemon to start..."
for i in $(seq 1 10); do
    if [ -S "$SOCKET" ]; then
        echo "Socket ready at $SOCKET"
        break
    fi
    sleep 1
done

if [ ! -S "$SOCKET" ]; then
    echo "ERROR: Socket not found after 10 seconds"
    exit 1
fi

# Check dashboard is up
if curl -s http://127.0.0.1:3030/api/status > /dev/null 2>&1; then
    echo "Dashboard running at http://127.0.0.1:3030"
else
    echo "WARNING: Dashboard not responding"
fi
echo ""

# Step 2: Test socket protocol (have query)
echo "--- Step 2: Test have query via socket ---"
echo "Testing with a synthetic store path..."

# socat approach: send mode header + have command, read response
RESPONSE=$(printf 'mode: query\nhave /gnu/store/00000000000000000000000000000000-hello\n' | \
    guix shell socat -- socat - UNIX-CONNECT:"$SOCKET" 2>/dev/null) || true

echo "Response: $(echo "$RESPONSE" | head -5)"
echo ""

# Step 3: Test with a real store path
echo "--- Step 3: Test with real store path ---"
HELLO_PATH=$(guix build hello --dry-run 2>&1 | head -1)

if [ -z "$HELLO_PATH" ]; then
    echo "WARNING: Could not determine hello store path"
    echo "Skipping real path test"
else
    echo "Store path: $HELLO_PATH"

    # Extract hash part (first 32 chars after /gnu/store/)
    HASH_PART=$(echo "$HELLO_PATH" | sed 's|^/gnu/store/||' | cut -c1-32)
    echo "Hash part: $HASH_PART"

    # Test have query with real path
    echo "Sending have query..."
    RESPONSE=$(printf "mode: query\nhave %s\n" "$HELLO_PATH" | \
        guix shell socat -- socat - UNIX-CONNECT:"$SOCKET" 2>/dev/null) || true
    echo "Have response: $(echo "$RESPONSE" | head -5)"
    echo ""

    # Test info query with real path
    echo "Sending info query..."
    RESPONSE=$(printf "mode: query\ninfo %s\n" "$HELLO_PATH" | \
        guix shell socat -- socat - UNIX-CONNECT:"$SOCKET" 2>/dev/null) || true
    echo "Info response:"
    echo "$RESPONSE" | head -20
    echo ""

    # Check for fd4: prefix in response
    if echo "$RESPONSE" | grep -q "^fd4:"; then
        echo "SUCCESS: Socket protocol has fd4: channel prefix"
    else
        echo "NOTE: Response does not have fd4: prefix (may be unfixed lines for direct socket)"
    fi
    echo ""
fi

# Step 4: Test the fd 4 relay protocol directly
echo "--- Step 4: Test relay mode with fd 4 ---"
echo "This simulates what guix-daemon does: write to stdin, read from fd 4."

# Create a temp file to capture fd 4 output
FD4_OUTPUT=$(mktemp)
FD4_FIFO=$(mktemp -u)
mkfifo "$FD4_FIFO"

# Start a background reader that writes the fifo to fd4_output
(cat "$FD4_FIFO" > "$FD4_OUTPUT" &)
CAT_PID=$!

# Open fd 4 pointing to the fifo
exec 4>"$FD4_FIFO"

echo "Testing --query --socket mode (writes to fd 4)..."
echo "have /gnu/store/00000000000000000000000000000000-test" | \
    "$BINARY" --query --socket "$SOCKET" 4>&1 || true

# Close fd 4 and wait for reader
exec 4>&-
sleep 1
kill "$CAT_PID" 2>/dev/null || true

if [ -s "$FD4_OUTPUT" ]; then
    echo "fd 4 output received:"
    cat "$FD4_OUTPUT"
else
    echo "No output on fd 4 (this may be expected if no providers found for synthetic path)"
fi

rm -f "$FD4_OUTPUT" "$FD4_FIFO"
echo ""

# Step 5: Test substitute policy behavior
echo "--- Step 5: Test substitute policy (p2p-first have query) ---"
echo "With p2p-first policy, 'have' should always reply yes (HTTP fallback available)..."

# Restart with p2p-first (already running)
if [ -n "$HELLO_PATH" ]; then
    RESPONSE=$(printf "mode: query\nhave %s\n" "$HELLO_PATH" | \
        guix shell socat -- socat - UNIX-CONNECT:"$SOCKET" 2>/dev/null) || true
    echo "p2p-first have response: $(echo "$RESPONSE" | head -3)"
fi
echo ""

# Step 6: Test p2p-only policy (should only reply if DHT providers exist)
echo "--- Step 6: Test p2p-only policy ---"
echo "Restarting daemon with --policy p2p-only..."

# Kill current daemon
kill "$DAEMON_PID" 2>/dev/null || true
wait "$DAEMON_PID" 2>/dev/null || true
DAEMON_PID=""
rm -f "$SOCKET"

"$BINARY" --daemon \
    --cache-dir "$CACHE_DIR" \
    --socket "$SOCKET" \
    --policy p2p-only \
    &

DAEMON_PID=$!
echo "New daemon PID: $DAEMON_PID (p2p-only policy)"

for i in $(seq 1 10); do
    if [ -S "$SOCKET" ]; then break; fi
    sleep 1
done

if [ -S "$SOCKET" ] && [ -n "$HELLO_PATH" ]; then
    RESPONSE=$(printf "mode: query\nhave %s\n" "$HELLO_PATH" | \
        guix shell socat -- socat - UNIX-CONNECT:"$SOCKET" 2>/dev/null) || true
    echo "p2p-only have response: $(echo "$RESPONSE" | head -3)"
    echo "(Should be empty or blank since no DHT providers exist on fresh daemon)"
fi
echo ""

echo "=== Pipe test complete ==="
echo "Dashboard (if still running): http://127.0.0.1:3030"
echo ""
echo "Use '$0 --cleanup' to stop the daemon and clean up."
echo "Press Ctrl+C to stop the daemon now, or it will be cleaned up on exit."
echo ""
echo "To manually interact with the daemon:"
echo "  socat - UNIX-CONNECT:$SOCKET"
echo "  Then type: mode: query"
echo "  Then type: have /gnu/store/<hash>-<name>"
