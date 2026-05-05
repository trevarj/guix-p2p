#!/bin/sh
# Standalone test suite for the guix wrapper script's interception logic.
# Run: scripts/test-wrapper.sh
#
# Does NOT require the real guix binary or guix-p2p-substitute.
# Uses a mock "guix" and mock "guix-p2p-substitute" to verify
# every invocation pattern is routed correctly.

set -eu

DIR="$(mktemp -d)"
trap 'rm -rf "$DIR"' EXIT

WRAPPER="$DIR/guix"
MOCK_REAL="$DIR/real-guix"
MOCK_P2P="$DIR/guix-p2p-substitute"
PASSED=0
FAILED=0

# ----- Mock binaries -----

cat > "$MOCK_REAL" << 'REAL'
#!/bin/sh
echo "REAL_GUIX: $*" >&2
printf "%s\n" "$*" >> "$0.out"
REAL
cat > "$MOCK_P2P" << 'P2P'
#!/bin/sh
echo "P2P_SUBSTITUTE: $*" >&2
printf "%s\n" "$*" >> "$0.out"
P2P
chmod +x "$MOCK_REAL" "$MOCK_P2P"

# Build wrapper with mock paths, fixing up REAL_GUIX
sed "s|/run/current-system/profile/bin/guix|$MOCK_REAL|" \
    "$(dirname "$0")/guix-wrapper.sh" > "$WRAPPER"
chmod +x "$WRAPPER"

# Add both mocks to PATH for the wrapper
export PATH="$DIR:$PATH"

# ----- Test helpers -----

assert_p2p() {
    local label="$1" cmd="$2" expected="$3"
    rm -f "$MOCK_P2P.out" "$MOCK_REAL.out"
    if eval "$cmd" >/dev/null 2>&1; then
        if [ -f "$MOCK_P2P.out" ]; then
            local got
            got=$(head -n1 "$MOCK_P2P.out")
            if [ "$got" = "$expected" ]; then
                PASSED=$((PASSED + 1))
                echo "  PASS: $label"
            else
                FAILED=$((FAILED + 1))
                echo "  FAIL: $label (got: '$got', expected: '$expected')"
            fi
        else
            FAILED=$((FAILED + 1))
            echo "  FAIL: $label (no p2p invocation, hit real guix instead)"
        fi
    else
        FAILED=$((FAILED + 1))
        echo "  FAIL: $label (command did not exit cleanly)"
    fi
}

assert_real() {
    local label="$1" cmd="$2"
    rm -f "$MOCK_P2P.out" "$MOCK_REAL.out"
    if eval "$cmd" >/dev/null 2>&1; then
        if [ -f "$MOCK_REAL.out" ]; then
            PASSED=$((PASSED + 1))
            echo "  PASS: $label"
        else
            FAILED=$((FAILED + 1))
            echo "  FAIL: $label (did NOT hit real guix)"
        fi
    else
        FAILED=$((FAILED + 1))
        echo "  FAIL: $label (command did not exit cleanly)"
    fi
}

# ----- Daemon substitute interception tests -----

echo "=== Daemon 'guix substitute' interception ==="

assert_p2p "substitute --query" \
    "guix substitute --query" \
    "--query"

assert_p2p "substitute --substitute" \
    "guix substitute --substitute" \
    "--substitute"

assert_p2p "substitute --query with extra args" \
    "guix substitute --query --bootstrap-peers foo" \
    "--query --bootstrap-peers foo"

assert_p2p "substitute --substitute with extra args" \
    "guix substitute --substitute --listen-addr bar" \
    "--substitute --listen-addr bar"

# ----- Non-substitute passthrough tests -----

echo ""
echo "=== Non-substitute commands pass through ==="

assert_real "build" \
    "guix build hello"

assert_real "install to profile" \
    "guix install emacs"

assert_real "system reconfigure" \
    "guix system reconfigure /etc/config.scm"

assert_real "home reconfigure" \
    "guix home reconfigure ~/.config/guix/home.scm"

assert_real "pull" \
    "guix pull"

assert_real "package search" \
    "guix package -s vim"

assert_real "no args (help)" \
    "guix"

assert_real "shell" \
    "guix shell python"

# The wrapper only matches "substitute" as first arg, so other subcommands
# starting with similar prefixes should NOT match.

assert_real "substitute-urls as flag (not subcommand)" \
    "guix build --substitute-urls=https://example.org hello"

# ----- Edge cases -----

echo ""
echo "=== Edge cases ==="

# "guix substitute" without --query/--substitute should pass through to real
assert_real "substitute without flag" \
    "guix substitute"

assert_real "substitute with unknown flag" \
    "guix substitute --help"

# ----- Results -----

echo ""
echo "=============================="
echo "  Passed: $PASSED"
echo "  Failed: $FAILED"
echo "=============================="
[ "$FAILED" -eq 0 ] || exit 1
