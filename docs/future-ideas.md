# Future Implementation Ideas

These are improvements that could be made after the core distribution mechanism
is stable and deployed.

## Background Health Monitoring

Currently, if DHT provider count drops below `min_providers`, the download
bails out and guix-daemon falls through to HTTP. A background task could
periodically scan seeded nars, check their DHT provider counts, and
proactively re-fetch at-risk nars from the HTTP substitute servers and
re-seed them via `NarStore::save()`. This would improve availability for
the next requester without any manual intervention.

## Parallel HTTP + P2P Fetching

Start downloading a nar from both the P2P swarm and the HTTP substitute
server simultaneously. Take whichever finishes first and cancel the other.
This reduces latency for the common case (HTTP is fast) while still
benefiting from P2P when HTTP is slow or unavailable.

Implementation: in `try_swarm_substitute`, start the HTTP nar download as a
concurrent tokio task. If the swarm download succeeds first, cancel the
HTTP task. If HTTP succeeds first, save to nar store and reply success.

## HTTP and Multi-Peer Benchmark Suite

Extend the benchmark harness to compare HTTP, p2p-only, and p2p-first modes
with repeated runs and multiple seed counts. The important comparison is not
the current single-node `hello` proof, but median and p95 elapsed time across
small, medium, and large packages with 1, 3, 5, and 8 seeders.

The methodology and acceptance criteria are documented in
`docs/benchmarks.md` under "Future Benchmark Work".

## Corporate/LAN Proxy Mode

A single guix-p2p daemon in an office can serve as a local substitute mirror.
Other machines on the LAN discover it via mDNS (already working) and pull
popular nars from it instead of hitting the remote substitute servers. This
is the highest-ROI use case for organizations running multiple Guix
machines behind a single internet connection.

## Seeding Priority for Heavy Packages

Large, slow-to-build packages (linux kernels, firefox, llvm, rust)
disproportionately benefit from P2P distribution. A config option like
`seed_patterns` could auto-seed nars matching certain store path patterns
(eg `*-linux-*,*-firefox-*,*-rust-*`), keeping them in the local nar store
and announced in the DHT without explicit `--seed` flags.

## Query Privacy

DHT lookups reveal which store paths a node is interested in. Future work
could add query padding, batch DHT lookups, or private information retrieval
techniques to reduce information leakage.

## Nar Request Batching

When `guix-daemon` sends a "have" query with many store paths, each one
triggers a separate DHT provider lookup. Batching these into a single DHT
batch query would reduce latency and DHT overhead for large substituter
queries.

## Re-fetch Missing Narinfo Cache Entries

On daemon startup, cached nars are already re-indexed, annotated with matching
local narinfo metadata, and re-announced. A future availability task could
iterate narinfo metadata whose NAR bytes are missing locally, optionally
re-fetch those nars from HTTP, and seed them proactively.
