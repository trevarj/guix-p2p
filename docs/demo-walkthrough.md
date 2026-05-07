# Demo Walkthrough Plan

## What We Need to Prove

1. guix-p2p speaks the guix-daemon substitute protocol correctly (fd 4)
2. The GUIX env var intercept works (no PATH hacks)
3. Two nodes can transfer a nar via P2P on the same machine
4. All three substitute policies work (p2p-only, p2p-first, http-first)
5. The dashboard shows real-time activity

## Prerequisites (must fix before demo)

### P1: Socket multiplexing for stdout traces

guix-daemon reads `@ download-started` and `@ download-succeeded` from fd 1
(stdout) of the substituter process. The socket bridge only carries fd 4 data.
The relay only writes to fd 4.

**Fix**: Channel prefix framing on the socket. Every message written by the
daemon gets a prefix:
- `fd4:<line>\n` — fd 4 structured data
- `out:<line>\n` — stdout trace messages

The relay demuxes: `fd4:` lines go to fd 4, `out:` lines go to stdout.

### P2: Emit `@ download-started` and `@ download-succeeded` traces

Even in direct mode, guix-p2p only emits `@ download-succeeded`. The
`format_trace_started` and `format_trace_progress` functions are dead code.

**Fix**: Call `format_trace_started` before download begins and
`format_trace_succeeded` after completion. Send as `out:` prefixed messages
over the socket (or stdout in direct mode).

### P3: Hash verification and `hash-mismatch` reply

After downloading (P2P or HTTP), verify computed nar hash against narinfo's
`nar_hash`. On mismatch: delete dest file, reply
`hash-mismatch <algo> <expected> <actual>`. On match: reply
`success sha256:<hash> <size>`.

The current code computes the hash and reports it as-is, without comparing
against the narinfo. The `hash-mismatch` reply type is never used.

### P4: Two-node same-machine testing

Two `guix-p2p --daemon` processes with different `--listen-addr`,
`--cache-dir`, `--socket`, `--dashboard-port`. mDNS may conflict (both
respond to the same queries). Each has a separate Ed25519 identity (already
handled — identity is per cache_dir).

## Guix-Daemon Protocol Reference

### How guix-daemon invokes the substitute binary

The daemon uses `settings.guixProgram` (default: `<nixBinDir>/guix`,
override via `GUIX` env var):

```
<guix> substitute --query        # persistent agent for have/info
<guix> substitute --substitute  # persistent agent for substitute
```

The daemon also sets `_NIX_OPTIONS` env var with packed settings that the
Guile substitute reads for substitute URLs etc. guix-p2p ignores this and
gets substitute URLs from its own config.

### fd layout in the child process

| fd | Purpose |
|----|---------|
| 0 (stdin) | daemon writes commands (have/info/substitute) |
| 1 (stdout) | trace messages (`@ download-started/succeeded`) |
| 2 (stderr) | log/diagnostic output |
| 4 | structured replies (have paths, info, success/not-found) |

### Query protocol

**have**: daemon writes `have <path1> <path2> ...\n`. Reply: each available
path on a separate line, terminated by blank line.

**info**: daemon writes `info <path1> ...\n`. Reply per path: store_path,
deriver, ref_count, refs, download_size, nar_size, then blank line.

### Substitute protocol

**substitute**: daemon writes `substitute <store-path> <dest>\n`. Reply:
`success sha256:<hash> <size>` or `hash-mismatch <algo> <expected> <actual>`
or `not-found`.

## Demo Steps

### Step 1: Manual pipe test (no guix-daemon)

Verify fd 4 protocol works by piping commands directly:

```sh
guix-p2p --daemon --cache-dir /tmp/guix-p2p-demo --dashboard &
sleep 3

# Test have query
echo "have /gnu/store/abcd-hello" | guix-p2p --query --socket /tmp/guix-p2p-demo/guix-p2p.sock

# Test info query (use real store path)
HELLO_PATH=$(guix build hello --dry-run 2>&1 | head -1)
echo "info $HELLO_PATH" | guix-p2p --query --socket /tmp/guix-p2p-demo/guix-p2p.sock
```

### Step 2: Two-node P2P on same machine

```sh
# Node A (seeder) — port 6881
guix-p2p --daemon --cache-dir /tmp/node-a --listen-addr /ip4/127.0.0.1/tcp/6881 \
  --socket /tmp/node-a/guix-p2p.sock --dashboard --dashboard-port 3030 &

# Get Node A's peer ID
PEER_ID=$(curl -s http://127.0.0.1:3030/api/status | jq -r .peer_id)

# Node B (downloader) — port 6882
guix-p2p --daemon --cache-dir /tmp/node-b --listen-addr /ip4/127.0.0.1/tcp/6882 \
  --socket /tmp/node-b/guix-p2p.sock --dashboard --dashboard-port 3031 \
  --bootstrap-peers "/ip4/127.0.0.1/tcp/6881/p2p/$PEER_ID" &
```

### Step 3: guix-daemon integration via GUIX env var

```sh
# Create or use existing wrapper script as the GUIX binary
# The wrapper translates: guix substitute --query → guix-p2p --query --socket $SOCKET

# Restart guix-daemon with GUIX pointing to the wrapper
# On Guix System, modify guix-daemon-service-type configuration

# Then: guix build hello → triggers p2p substitute flow
guix build hello
```

### Step 4: Policy demos

- `p2p-only`: remove bootstrap peers, guix build fails gracefully
- `p2p-first`: default, show P2P then HTTP fallback
- `http-first`: show HTTP first, P2P fallback

### Step 5: Dashboard walkthrough

- Show seeds panel, builds panel, real-time events

## Implementation Order

1. Fix socket protocol (channel prefix framing) + relay demux
2. Add `@ download-started` / `@ download-succeeded` trace emission
3. Add hash verification + `hash-mismatch` reply
4. Manual pipe test (Step 1) to verify protocol correctness
5. Two-node P2P test (Step 2)
6. GUIX env var integration + guix build test (Step 3)
7. Policy demos (Step 4)
8. Write automated demo script
