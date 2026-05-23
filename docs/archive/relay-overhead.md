# Relay Startup Overhead

Archived: resolved by replacing the normal Guix extension relay path with a
Scheme socket client. The Rust `--query --socket` / `--substitute --socket`
relay remains available for compatibility and debugging.

Historical context: `guix-p2p` kept the expensive P2P state in a long-running
daemon, but the Guix substitute extension still started a short-lived
`guix-p2p` relay process for substitute protocol calls when the daemon socket
existed.

This document tracked whether that relay process startup was negligible or
worth removing.

## Old Flow

```
guix-daemon
  -> Guix substitute extension
  -> guix-p2p --query/--substitute --socket /var/cache/guix-p2p/guix-p2p.sock
  -> long-running guix-p2p --daemon
```

The daemon keeps these warm:

- libp2p swarm
- peer connections
- DHT/provider cache
- narinfo cache
- seed catalog
- dashboard state

Each relay invocation still pays for:

- Rust process startup
- CLI parsing and relay-mode setup
- minimal logging/config initialization
- Unix socket connect
- channel framing between relay and daemon
- NAR copy from daemon to relay to the requested destination file

## Questions To Answer

- Does Guix keep substitute helper processes alive for batches, or does it spawn
  one relay process per query/substitute request?
- What is the median and p95 latency added by the relay when no download occurs?
- How much extra time is spent copying NAR bytes through the relay for small,
  medium, and large NARs?
- Does this overhead materially affect `guix pull`, `guix shell`, or system
  builds?
- Is relay overhead larger than normal substitute server latency variance?

## Measurement Plan

Measure these separately:

- `guix-p2p --query --socket`: process startup plus socket round trip.
- `guix-p2p --substitute --socket`: relay process plus destination NAR write.
- Warm daemon socket round trip without starting a Rust relay process.
- Direct `guix-p2p --substitute` mode if still supported.
- Built-in `guix substitute --query` and `guix substitute --substitute`.
- Real `guix pull` or `guix build` with process spawn counts and elapsed time.

Use at least these NAR sizes:

- tiny: catches fixed startup cost
- medium: representative package substitute
- large: exposes byte-copy overhead

Report:

- invocation count
- median
- p95
- min/max
- total elapsed
- bytes transferred
- selected policy
- whether data came from P2P or HTTP fallback

Do not check benchmark result markdown into git. Store generated results as CI
artifacts or site data that can be regenerated.

## Decision Thresholds

Treat the relay as acceptable for now if:

- fixed relay overhead is below roughly 20-50 ms per invocation
- Guix batches enough requests that process startup is not per store item
- total `guix pull` or `guix build` impact is lost in normal network variance

Prioritize removing the relay process if:

- process startup is hundreds of ms per invocation
- Guix starts a relay process for every store item
- p95 overhead is visible in real `guix pull` runs
- small substitutes are dominated by relay startup rather than network or NAR
  restore time

Prioritize byte-path changes if:

- fixed startup overhead is acceptable
- large NARs spend significant time in daemon-to-relay-to-destination copying

## Candidate Fixes

### Scheme Socket Client

Move the relay client into `guix/extensions/substitute.scm`.

The extension would:

- connect to the daemon Unix socket directly
- send `mode: query` or `mode: substitute`
- write `fd4:` responses to fd 4
- write `out:` trace lines to stdout
- decode `nar:` chunks into the destination NAR file

This removes Rust process startup while preserving the warm Rust daemon.

### Persistent Relay Mode

Keep the Rust relay process alive for multiple substitute protocol lines when
Guix does so. This only helps if Guix reuses the substituter process for batches.

### Cheaper Rust Relay Startup

Keep the Rust relay but make relay mode avoid all unnecessary work:

- no identity loading
- no config loading beyond socket path and mode flags
- no tracing subscriber unless explicitly requested
- no network or swarm initialization

### Direct Daemon Destination Writes

Avoid the relay byte copy by letting the daemon write the destination NAR file.
This is risky because the daemon may not run with the same permissions and store
context as the substituter process spawned by `guix-daemon`.

## Preferred Path

First measure. If overhead matters, prefer a Scheme socket client in the Guix
substitute extension. It removes the extra Rust process without changing the
P2P daemon architecture or permission model.
