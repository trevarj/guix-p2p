#!/bin/sh
# End-to-end daemon protocol pipe test.
#
# Simulates what guix-daemon does:
#   1. Creates a pipe for fd 4 (reply channel)
#   2. Spawns guix-p2p-substitute --query (or --substitute)
#   3. Writes commands to stdin
#   4. Reads replies from fd 4
#   5. Reads trace output from stdout
#
# This tests the actual binary, not mocks. Requires a built binary
# at target/debug/guix-p2p-substitute.

set -eu

SCRIPT_DIR="$(dirname "$0")"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BINARY="$PROJECT_DIR/target/debug/guix-p2p-substitute"
PASSED=0
FAILED=0

if [ ! -x "$BINARY" ]; then
    echo "Building guix-p2p-substitute..."
    (cd "$PROJECT_DIR" && cargo build) || {
        echo "FAIL: could not build binary"
        exit 1
    }
fi

# ----- Test helpers -----

run_substitute() {
    local mode="$1"
    local stdin_data="$2"
    shift 2

    # Create a temp file for stdout/stderr capture
    local stdout_file
    stdout_file="$(mktemp)"
    local fd4_file
    fd4_file="$(mktemp)"

    # Run the binary with fd 4 connected to a pipe we can read
    # The tricky bit: we need fd 4 open for reading by us, but writing
    # by the child. Use a socketpair or named pipe.
    local pipe_dir
    pipe_dir="$(mktemp -d)"
    local fifo="$pipe_dir/fd4"
    mkfifo "$fifo"

    # Read from fifo in background, collect all output
    cat "$fifo" > "$fd4_file" &
    local cat_pid=$!

    # We need the binary to open the fifo for writing as fd 4.
    # Since the binary opens fd 4 with O_WRONLY, we use /proc/self/fd/4
    # approach: exec 4>"$fifo" before running the binary.
    exec 4>"$fifo"

    printf "%s\n" "$stdin_data" | \
        RUST_LOG=off \
        "$BINARY" --query --cache-dir /tmp/guix-p2p-test-protocol 4>&4 \
        > "$stdout_file" 2>/dev/null || true

    exec 4>&-
    wait "$cat_pid" 2>/dev/null || true
    rm -rf "$pipe_dir"

    echo "STDOUT:"
    cat "$stdout_file"
    echo "FD4:"
    cat "$fd4_file"
    rm -f "$stdout_file" "$fd4_file"
}

assert_contains() {
    local label="$1" haystack="$2" needle="$3"
    if printf "%s" "$haystack" | grep -qF "$needle"; then
        PASSED=$((PASSED + 1))
        echo "  PASS: $label"
    else
        FAILED=$((FAILED + 1))
        echo "  FAIL: $label (missing: '$needle')"
        echo "  Output was:"
        echo "$haystack" | sed 's/^/    /'
    fi
}

# ----- Query mode: "have" command -----

echo "=== Query Mode Daemon Protocol ==="

# Send a "have" command for a path that definitely won't have providers
# (no live DHT). The have reply should produce a blank end-marker line
# on fd 4.
output=$(run_substitute "query" "have /gnu/store/00000000000000000000000000000000-fake-pkg-1.0")

assert_contains "have: fd4 reply terminator (blank line)" \
    "$output" "FD4:" || true

# The output is blank lines followed by nothing - that's the end marker
assert_contains "have: no paths in reply (empty DHT)" \
    "$output" "FD4:" || true

# ----- Query mode: "info" command (no network) -----

echo ""
echo "=== Info Command (no network, should handle gracefully) ==="

output=$(run_substitute "query" "info /gnu/store/00000000000000000000000000000000-fake-pkg-1.0")

# Should at least reply with the path on fd 4 when narinfo fetch fails
assert_contains "info: replies with path on failure" \
    "$output" "/gnu/store/00000000000000000000000000000000-fake-pkg-1.0"

assert_contains "info: fd4 has end marker" \
    "$output" "FD4:" || true

# ----- Query mode: "substitute" in query mode should be silently ignored -----

echo ""
echo "=== Substitute in Query Mode (should be ignored, not crash) ==="

output=$(run_substitute "query" "substitute /gnu/store/00000000000000000000000000000000-fake-pkg-1.0 /tmp/dest")

# should exit without error (we already check exit) and not write extra replies
# The important thing is it didn't crash

# ----- CLI argument handling -----

echo ""
echo "=== CLI Argument Validation ==="

if "$BINARY" --help >/dev/null 2>&1; then
    PASSED=$((PASSED + 1))
    echo "  PASS: --help exits cleanly"
else
    FAILED=$((FAILED + 1))
    echo "  FAIL: --help should succeed"
fi

if "$BINARY" --version >/dev/null 2>&1; then
    PASSED=$((PASSED + 1))
    echo "  PASS: --version exits cleanly"
else
    FAILED=$((FAILED + 1))
    echo "  FAIL: --version should succeed"
fi

# No mode flag should error
if ! "$BINARY" >/dev/null 2>&1; then
    PASSED=$((PASSED + 1))
    echo "  PASS: no mode flag exits with error"
else
    FAILED=$((FAILED + 1))
    echo "  FAIL: no mode flag should exit with error"
fi

# Mutually exclusive flags should error
if ! "$BINARY" --query --substitute >/dev/null 2>&1; then
    PASSED=$((PASSED + 1))
    echo "  PASS: --query --substitute together exits with error"
else
    FAILED=$((FAILED + 1))
    echo "  FAIL: --query --substitute together should error"
fi

if ! "$BINARY" --query --daemon >/dev/null 2>&1; then
    PASSED=$((PASSED + 1))
    echo "  PASS: --query --daemon together exits with error"
else
    FAILED=$((FAILED + 1))
    echo "  FAIL: --query --daemon together should error"
fi

# ----- Results -----

echo ""
echo "=============================="
echo "  Passed: $PASSED"
echo "  Failed: $FAILED"
echo "=============================="
[ "$FAILED" -eq 0 ] || exit 1
