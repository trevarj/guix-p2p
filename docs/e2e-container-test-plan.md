# E2E Container Test Plan

Current status: `guix-p2p-e2e container-smoke` is the canonical implementation.
`scripts/e2e-container-test.sh` is a thin compatibility wrapper around that
Rust harness. The harness runs Node A, Node B, Node B's raw `guix-daemon`, and
the client `guix build` through `guix shell -CN` containers with generated TOML
config, captured logs, and dashboard API validation. On hosts with a read-only
store, use the disposable VM in `docs/e2e-vm.md`.

## Goal

Run multiple `guix shell -CN` containers, each with its own `guix-daemon` +
`guix-p2p` daemon. Node A seeds a package (e.g. `hello`). Node B runs `guix
build hello` through guix-daemon, which uses the P2P substituter via the `GUIX`
env var, and successfully downloads the nar from Node A via P2P.

Policy: `p2p-only` (no HTTP fallback -- pure P2P). The checked-in
orchestrator is `cargo run -p guix-p2p-e2e -- container-smoke`.

The compatibility wrapper `scripts/e2e-container-test.sh` prints timestamped
progress and captures wrapper output under `target/guix-p2p-e2e-logs/` by
default:

```sh
target/guix-p2p-e2e-logs/e2e-container-test.log
target/guix-p2p-e2e-logs/e2e-container-test-output.log
```

Set `GUIX_P2P_E2E_LOG_DIR` to move these logs, or
`GUIX_P2P_E2E_HEARTBEAT_SECS` to change the default 30-second heartbeat. The
Rust harness still writes node and build logs under `$GUIX_P2P_E2E_BASE/logs/`.

## Container Approach

**`guix shell -CN`** -- lightweight containers with filesystem isolation (`-C`)
and host network access (`-N`).

- Shared network: all containers reach each other on `127.0.0.1` via different
  TCP ports by default. Set `GUIX_P2P_E2E_TRANSPORT=quic` to test QUIC UDP.
- Filesystem isolation: separate root, but the harness shares the project
  checkout, generated E2E base directory, and writable `/gnu/store`.
- No root needed. Fast startup. Easy cleanup.

The dashboard-first prototype boots a full qcow2 Guix image with
`scripts/e2e-vm.sh run`, shares the static payload into the guest, and runs
`container-smoke --dashboard-bind 0.0.0.0 --vm-direct` there. This avoids
`guix system vm`, which shares the host store and can inherit a read-only
`/gnu/store`.

## Store Strategy: Separate DB, Shared Store

The host's `/gnu/store` is shared into containers as writable. Each
container has its own `GUIX_STATE_DIRECTORY` with a **separate empty DB**. The
container's guix-daemon doesn't know any packages are valid, even if they
exist on disk in `/gnu/store`. When `guix build hello` runs, the daemon sees
hello isn't valid in its DB and asks the substituter to provide it.

| Path | Strategy | Rationale |
|------|----------|-----------|
| `/gnu/store` | Shared from host (writable) | guix-daemon needs to write nar imports; the store itself is fine to share |
| `/var/guix` (state) | Per-node `GUIX_STATE_DIRECTORY` | Separate DBs, GC roots, temp roots per node |
| `/etc/guix` (config) | Per-node `GUIX_CONFIGURATION_DIRECTORY` | Separate ACL; copied from host at setup |

The harness preflights `guix shell -CN --writable-root --share=/gnu/store`
before starting nodes. If `/gnu/store` is read-only on the host or cannot be
shared writable into the container, the test stops immediately.

### Why not read-only store?

guix-daemon needs to write to the store when importing substituted nars. With
`--max-jobs=0` (substitutes only, no local builds), the daemon still writes
imported paths to the store. A read-only store would break substitution.

### Why not a separate NIX_STORE_DIR?

Overriding `NIX_STORE_DIR` changes the store root, but Guix programs hardcode
`/gnu/store/...` in store paths, derivations, and the substitute protocol.
Using a different store root would break path resolution. Sharing the host's
`/gnu/store` with a separate DB is the simplest viable approach.

## How `GUIX` Env Var Intercepts Substitutes

The C++ `guix-daemon` binary reads `settings.guixProgram = getEnv("GUIX",
nixBinDir + "/guix")` from `nix/libstore/globals.cc`. Every time it needs a
substituter, it runs `execv(settings.guixProgram, {"substitute", "--query",
...})`.

**Critical**: The Guile wrapper script at
`~/.config/guix/current/bin/guix-daemon` **unconditionally overwrites** the
`GUIX` env var with `(setenv "GUIX" "/gnu/store/...-guix-command")`. To keep
our custom `GUIX`, we must run the **raw C++ binary** directly, not the Guile
wrapper.

The raw C++ binary is at:
```
/gnu/store/<hash>-guix-<version>/bin/guix-daemon
```
(ELF binary, not the Guile wrapper)

## Architecture

```
Host network (shared by all CN containers)
|
+-- Node A (seeder) -- guix shell -CN
|   guix-p2p --daemon
|     --listen-addr /ip4/127.0.0.1/tcp/6881
|     --cache-dir /tmp/guix-p2p-e2e/node-a
|     --socket /tmp/guix-p2p-e2e/node-a.sock
|     --dashboard --dashboard-port 3031
|     --policy p2p-only
|
|   After ready: seed /gnu/store/...-hello
|
+-- Node B (builder/downloader) -- guix shell -CN
|   guix-p2p --daemon
|     --listen-addr /ip4/127.0.0.1/tcp/6882
|     --cache-dir /tmp/guix-p2p-e2e/node-b
|     --socket /tmp/guix-p2p-e2e/node-b.sock
|     --dashboard --dashboard-port 3032
|     --policy p2p-only
|     --bootstrap-peers /ip4/127.0.0.1/tcp/6881/p2p/<NODE_A_PEERID>
|
|   guix-daemon (raw C++ binary)
|     GUIX=/tmp/guix-p2p-e2e/node-b/guix-wrapper.sh
|     GUIX_STATE_DIRECTORY=/tmp/guix-p2p-e2e/node-b/state
|     GUIX_CONFIGURATION_DIRECTORY=/tmp/guix-p2p-e2e/node-b/etc
|     --disable-chroot
|     --max-jobs=0
|     --listen=/tmp/guix-p2p-e2e/node-b/daemon.sock
|
|   guix build hello  -->
|     guix-daemon spawns $GUIX substitute --query
|     -> guix-wrapper.sh
|     -> guix-p2p --query --socket /tmp/.../node-b.sock
|     -> P2P download from Node A
```

## Wrapper Script

The wrapper intercepts `guix substitute --query` and `guix substitute
--substitute` and routes them to `guix-p2p` relay mode. All other invocations
fall through to the real `guix` binary.

```sh
#!/bin/sh
# guix-p2p-wrapper.sh
# Intercepts guix substitute calls and routes to guix-p2p daemon

SOCKET="${GUIX_P2P_SOCKET:?GUIX_P2P_SOCKET not set}"
GUIX_P2P="${GUIX_P2P_BIN:?GUIX_P2P_BIN not set}"
REAL_GUIX="${REAL_GUIX_BIN:-/run/current-system/profile/bin/guix}"

case "${1-}" in
    substitute)
        shift
        case "${1-}" in
            --query|--substitute)
                if [ -S "$SOCKET" ]; then
                    exec "$GUIX_P2P" "$@" --socket "$SOCKET"
                else
                    exec "$REAL_GUIX" substitute "$@"
                fi
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
```

Env vars for the wrapper:
- `GUIX_P2P_SOCKET`: Path to the guix-p2p daemon's Unix socket
- `GUIX_P2P_BIN`: Absolute path to the `guix-p2p` binary
- `REAL_GUIX_BIN`: Absolute path to the real `guix` command (for fallthrough)

## Phase-by-Phase Execution

### Phase 1: Setup

```sh
BASE=/tmp/guix-p2p-e2e
rm -rf "$BASE" && mkdir -p "$BASE"

# Build guix-p2p
cargo build --release
GUIX_P2P_BIN=$(readlink -f target/release/guix-p2p)

# Find the raw C++ guix-daemon binary (not the Guile wrapper)
RAW_DAEMON=$(find /gnu/store -maxdepth 3 -name guix-daemon -type f \
  -path '*/bin/guix-daemon' -executable 2>/dev/null | \
  while read f; do head -c4 "$f" | grep -q $'\x7fELF' && echo "$f" && break; done)

# Find the real guix command for wrapper fallthrough
REAL_GUIX_BIN=$(readlink -f /run/current-system/profile/bin/guix)
```

### Phase 2: Start Node A (seeder)

```sh
mkdir -p "$BASE/node-a"

# Start guix-p2p daemon in a container
guix shell -CN --share="$BASE/node-a" \
  --expose=/gnu/store --expose=/var/guix --expose=/etc/guix \
  -- guix-p2p --daemon \
    --listen-addr /ip4/127.0.0.1/tcp/6881 \
    --cache-dir "$BASE/node-a" \
    --socket "$BASE/node-a.sock" \
    --dashboard --dashboard-port 3031 \
    --policy p2p-only \
    &

# Wait for readiness (poll dashboard API)
for i in $(seq 1 30); do
  if curl -s http://127.0.0.1:3031/api/status >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

# Capture Node A's PeerId
NODE_A_PEERID=$(curl -s http://127.0.0.1:3031/api/status | jq -r .peer_id)

# Seed hello on Node A
guix-p2p --seed /gnu/store/ab584kfyc7pymc1cmdrkwzz3lwv86yf6-hello-2.12.3 \
  --socket "$BASE/node-a.sock"
```

### Phase 3: Start Node B (builder/downloader)

```sh
mkdir -p "$BASE/node-b/state/db" "$BASE/node-b/state/daemon-socket" \
  "$BASE/node-b/state/gcroots" "$BASE/node-b/state/profiles" \
  "$BASE/node-b/state/substitute" "$BASE/node-b/state/temproots" \
  "$BASE/node-b/state/userpool" "$BASE/node-b/etc"

# Copy ACL from host for narinfo signature verification
cp /etc/guix/acl "$BASE/node-b/etc/acl" 2>/dev/null || true

# Create wrapper script
cat > "$BASE/node-b/guix-wrapper.sh" << EOF
#!/bin/sh
SOCKET="$BASE/node-b.sock"
GUIX_P2P="$GUIX_P2P_BIN"
REAL_GUIX="$REAL_GUIX_BIN"
case "\${1-}" in
  substitute) shift
    case "\${1-}" in
      --query|--substitute)
        if [ -S "\$SOCKET" ]; then
          exec "\$GUIX_P2P" "\$@" --socket "\$SOCKET"
        else
          exec "\$REAL_GUIX" substitute "\$@"
        fi ;;
      *) exec "\$REAL_GUIX" substitute "\$@" ;;
    esac ;;
  *) exec "\$REAL_GUIX" "\$@" ;;
esac
EOF
chmod +x "$BASE/node-b/guix-wrapper.sh"

# Start guix-p2p daemon in a container
guix shell -CN --share="$BASE/node-b" \
  --expose=/gnu/store --expose=/var/guix --expose=/etc/guix \
  -- guix-p2p --daemon \
    --listen-addr /ip4/127.0.0.1/tcp/6882 \
    --cache-dir "$BASE/node-b" \
    --socket "$BASE/node-b.sock" \
    --dashboard --dashboard-port 3032 \
    --policy p2p-only \
    --bootstrap-peers "/ip4/127.0.0.1/tcp/6881/p2p/${NODE_A_PEERID}" \
    &

# Wait for readiness
for i in $(seq 1 30); do
  if curl -s http://127.0.0.1:3032/api/status >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

# Start guix-daemon with separate state and GUIX override
GUIX="$BASE/node-b/guix-wrapper.sh" \
GUIX_STATE_DIRECTORY="$BASE/node-b/state" \
GUIX_CONFIGURATION_DIRECTORY="$BASE/node-b/etc" \
"$RAW_DAEMON" \
  --disable-chroot \
  --max-jobs=0 \
  --listen="$BASE/node-b/daemon.sock" \
  &
```

### Phase 4: Build hello via P2P

```sh
# Tell guix to use the container's daemon socket
GUIX_DAEMON_SOCKET="$BASE/node-b/daemon.sock" \
  guix build hello
```

### Phase 5: Verify

```sh
# Check Node B's dashboard for catalog entry and download events
curl -s http://127.0.0.1:3032/api/catalog | jq .
curl -s http://127.0.0.1:3032/api/builds | jq .

# Check Node A's dashboard for block-served events (via WebSocket)
# (manual browser check at http://127.0.0.1:3031)

# Check Node B's nar cache
ls "$BASE/node-b/nar/"

# Verify build exit code
echo "Build exit code: $?"
```

### Phase 6: Cleanup

```sh
# Kill all background processes
jobs -p | xargs kill 2>/dev/null

# Remove test directory
rm -rf "$BASE"
```

## Two Separate Substitute URL Configs

There are two substitute URL configurations that must be understood:

1. **guix-daemon's `--substitute-urls`**: What the daemon tells the
   substituter via `_NIX_OPTIONS`. Used for `have`/`info` queries. Should be
   `https://bordeaux.guix.gnu.org https://ci.guix.gnu.org` (defaults) since
   guix-p2p needs narinfo metadata from these servers.

2. **guix-p2p's `--substitute-urls` / config**: What guix-p2p uses to fetch
   narinfol for hash verification. With `--policy p2p-only`, narinfos are
   fetched from these URLs but nars are downloaded exclusively via P2P.

The daemon's `--substitute-urls` should remain the defaults. guix-p2p
intercepts the protocol and applies its own policy for nar download.

## Port Assignments

| Node | QUIC port | Dashboard port | Socket path | Daemon socket |
|------|-----------|----------------|-------------|---------------|
| A (seeder) | 6881/tcp | 3031 | `$BASE/node-a.sock` | N/A |
| B (builder) | 6882/tcp | 3032 | `$BASE/node-b.sock` | `$BASE/node-b/daemon.sock` |

For 3+ nodes, increment ports: 6883, 3033, node-c, etc.

## Known Challenges

### PeerId capture for bootstrap

The seeder's PeerId is generated/loaded from its cache-dir keypair on startup.
We can't know it before the daemon starts. Poll the dashboard `/api/status`
endpoint after startup to get the `peer_id` field, then construct the bootstrap
multiaddr: `/ip4/127.0.0.1/tcp/6881/p2p/<PeerId>`.

### mDNS across containers

mDNS uses multicast UDP which may not propagate across containers even with
shared network namespace. Explicit `--bootstrap-peers` is the reliable
approach. Do not rely on mDNS for container tests.

### DHT convergence requires 3+ nodes

Kademlia DHT routing table convergence on a LAN requires at least 3 nodes. For
the 2-node test (A + B), DHT discovery may fail. Workaround: Node B directly
dials Node A via `--bootstrap-peers`, which establishes a direct connection and
DHT query goes to the known peer. For full DHT tests, add a 3rd node.

### guix-daemon --disable-chroot

Required for running inside a container because nested user namespaces may not
be available. Safe when only doing substitutes (no local builds) since the
container provides the isolation that chroot would normally provide.

### guix build may already have hello cached

If the host's guix-daemon already built hello, it's in `/gnu/store`. The
container has its own DB (empty), so it doesn't know hello is valid. The
container's daemon should ask the substituter. But if the substituter
(guix-p2p) replies "have hello" (which it does in `p2p-first`/`http-first`
mode), the daemon will try to substitute it. The nar write to `/gnu/store` may
be a no-op if the file already exists, and the DB is updated. This should
work.

However, if something goes wrong, consider using a package that is NOT in the
host store, or deleting it first: `guix gc --delete
/gnu/store/ab584kfyc7pymc1cmdrkwzz3lwv86yf6-hello-2.12.3` (dangerous -- only
if nothing depends on it).

### GUIX_STATE_DIRECTORY structure

The guix-daemon expects this directory structure under `GUIX_STATE_DIRECTORY`:

```
state/
  daemon-socket/socket
  db/
  gcroots/
  profiles/
  substitute/
  temproots/
  userpool/
```

The script must create these directories before starting guix-daemon. The
daemon may auto-create some, but not all.

### ACL / signing keys

guix-p2p needs `/etc/guix/acl` (or `GUIX_CONFIGURATION_DIRECTORY/acl`) for
narinfo signature verification. Copy from the host at setup time. The
container's `GUIX_CONFIGURATION_DIRECTORY` must contain a valid `acl` file.

For the host's substitute servers to be trusted, the ACL must include the
official Guix signing keys. On Guix System, `/etc/guix/acl` already has these.

## Future Enhancements

- 3-node test for full DHT convergence
- `guix system container` approach with Shepherd-managed services
- Automated CI via `guix shell -CN` (no Docker dependency)
- NAT traversal testing (separate network namespaces with `ip netns`)
- HTTP fallback testing with `--policy p2p-first`
- Dashboard screenshot comparison for visual regressions
