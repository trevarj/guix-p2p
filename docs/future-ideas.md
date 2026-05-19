# Future Implementation Ideas

These are improvements that could be made after the core distribution mechanism
is stable and deployed.

## Background Health Monitoring

The daemon now periodically checks provider counts for local seeds and
re-announces them when known providers fall below `min_providers`. A future
availability task could make this more proactive by re-fetching missing
at-risk nars from HTTP and saving them through `NarStore::save()`.

## P2P-First Latency Tuning

Avoid racing HTTP and P2P for the same nar because duplicate downloads waste
bandwidth. Latency work should instead improve the existing P2P-first path:
provider health scoring, faster stale-provider pruning, startup re-seeding,
and benchmark-driven scheduler tuning. HTTP fallback should remain a fallback,
not a parallel duplicate transfer.

## HTTP and Multi-Peer Benchmark Suite

Extend the benchmark harness to compare HTTP, p2p-only, and p2p-first modes
with repeated runs and multiple seed counts. The important comparison is not
the current single-node `hello` proof, but median and p95 elapsed time across
small, medium, and large packages with 1, 3, 5, and 8 seeders.

The methodology and acceptance criteria are documented in
`docs/benchmarks.md` under "Future Benchmark Work".

## Async HTTP Decompression

The current HTTP fallback downloads the compressed response body into memory,
then decompresses it into a raw NAR buffer before hash verification and restore.
This is simple and keeps lzip support through `lzma-rust2`, but it temporarily
holds both compressed and decompressed bytes.

Revisit `async-compression` when HTTP downloads become common in benchmark
evidence or when large-NAR HTTP fallback memory use becomes a practical issue.
The likely first step is streaming gzip and zstd through async decoders while
keeping lzip on the existing `lzma-rust2` path until async lzip support is
available. The refactor should preserve per-candidate hash verification,
progress traces, bandwidth limiting, and fallback to lower-preference
compression candidates.

## Corporate/LAN Proxy Mode

A single guix-p2p daemon in an office can serve as a local substitute mirror.
Other machines on the LAN discover it via mDNS (already working) and pull
popular nars from it instead of hitting the remote substitute servers. This
is the highest-ROI use case for organizations running multiple Guix
machines behind a single internet connection.

## Full-P2P Attestations

Full-P2P mode needs decentralized substitute metadata, not just P2P NAR bytes.
The future plan is documented in
[full-p2p-attestations.md](full-p2p-attestations.md): reuse Guix
publish-style signing keys, require explicit threshold trust, and fail closed.

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
