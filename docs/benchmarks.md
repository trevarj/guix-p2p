# E2E and Benchmarks

## Container Smoke

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- container-smoke --package hello --transport tcp
```

For a dashboard-first demo that keeps the validated nodes running:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- container-smoke \
  --package hello \
  --transport tcp \
  --dashboard-bind 0.0.0.0 \
  --hold \
  --keep-temp
```

Defaults:

- package: `hello`
- transport: TCP loopback
- base: `/tmp/guix-p2p-e2e`
- Node A dashboard: `3031`
- Node B dashboard: `3032`
- dashboard bind: `127.0.0.1`

Use QUIC on hosts that allow UDP:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- container-smoke --package hello --transport quic
```

The harness:

- builds or locates `target/release/guix-p2p`
- resolves a real store path with `guix build <package>`
- resolves the raw ELF `guix-daemon`
- preflights `guix shell -CN` with writable `/gnu/store`
- starts Node A in a Guix container with the resolved store path seeded
- starts Node B in a separate Guix container with `substitute_policy = "p2p-only"` and `min_providers = 1`
- runs the copied substitute extension directly for `--query` and `--substitute`
  with `GUIX_P2P_SUBSTITUTE_ROUTING=p2p-only`
- verifies Node B claims the seeded path and restores it from P2P
- captures logs under `$BASE/logs/`
- with `--hold`, keeps daemons and dashboards alive after validation until Ctrl-C

When `--guix-p2p-bin` points at a Guix store item, the harness exposes that
store item plus its runtime libraries inside each disposable container. The P2P
node containers include `gcc-toolchain` so dynamically linked Rust binaries can
load `libgcc_s.so.1` reliably.

The container smoke intentionally avoids a full isolated `guix-daemon` build.
The isolated daemon can spawn `guix substitute --query` and then keep the
substituter idle without sending a query line, which makes the container smoke
an unreliable oracle for daemon integration. Use the VM e2e for that path.

Acceptance checks:

- Direct extension `--query` reports the seeded path.
- Direct extension `--substitute` restores the seeded path from P2P.
- Node A `/api/seeds` includes the seeded nar.
- Node B `/api/catalog` includes the requested store path or nar hash.
- Node A logs show block serving.
- Node B logs show p2p-only handling and a successful substitute download.
- Node B logs do not show HTTP nar fallback in p2p-only mode.

CI runs this TCP container smoke on every push and pull request after formatting,
clippy, and the workspace test suite. The CI smoke uses `sl` because it is small
enough for fast hosted-runner feedback while still exercising a real Guix store
path, nar hash, query reply, P2P block transfer, and substitute restore. The
container smoke seeds only the requested output path, not its full closure, to
keep hosted-runner startup small. Before probing substitution, the harness waits
for Node B to see the seed as a connected peer and retries the direct `have`
query long enough for Kad provider discovery to converge. If that readiness
step fails, the harness prints Node B status/catalog, seed status/seeds, and the
query, substitute, seed, and fetcher log tails. CI sets
`GUIX_P2P_E2E_DASHBOARD_TIMEOUT_SECS=600` so cold Guix profile downloads do not
fail the smoke before the seed dashboard starts. CI also sets
`GUIX_P2P_E2E_NO_GUIX_SHELL=1` because GitHub-hosted runners reject the nested
mount namespaces used by `guix shell -C`; the outer manifest still supplies the
Guix and native runtime dependencies. In this mode, the harness points manual
seeding at the Guile interpreter embedded in the active `guix` script so the
NAR serializer uses Guile modules from the same Guix profile.

## E2E VM Proof

The strict proof requires separate writable stores so a fetcher can prove it
does not already have the package seeded by another node. Shared host-store
containers are not a valid full proof for that requirement.

Use `cargo run -p guix-p2p-e2e -- vm ...` for the strict named-node VM proof
with separate writable stores. The VM proof currently passes for `hello`
through a fetcher node's raw extension-enabled `guix-daemon` path and verifies
that the imported store path is a restored directory.

## Benchmark

The authoritative benchmark path is the GitHub Actions VM workflow. Do not run
long VM benchmark suites on a developer workstation unless explicitly debugging
the harness. Dispatch CI instead:

- GitHub Actions: <https://github.com/trevarj/guix-p2p/actions/workflows/benchmarks.yml>
- Choose `suite`, `iterations`, and `modes`.
- Download the `guix-p2p-benchmark-*` artifact after the run completes.

For local harness debugging only, first complete the setup from `docs/e2e.md`:

```sh
cargo run -p guix-p2p-e2e -- vm image
cargo run -p guix-p2p-e2e -- vm run Bootstrap
cargo run -p guix-p2p-e2e -- vm run Alice
cargo run -p guix-p2p-e2e -- vm run Bob
cargo run -p guix-p2p-e2e -- vm wait-ssh Bootstrap Alice Bob
cargo run -p guix-p2p-e2e -- vm push-binary --all
cargo run -p guix-p2p-e2e -- vm bootstrap Bootstrap
```

Then run the VM benchmark:

```sh
cargo run -p guix-p2p-e2e -- vm benchmark \
  --suite system-build \
  --modes http,p2p-only,p2p-first,http-first \
  --http-conditions normal \
  --seed-nodes Alice \
  --fetch-node Bob \
  --http-node Bob \
  --iterations 1
```

The available suites are:

- `smoke`: quick `hello` check.
- `standard`: `hello`, `git`, and `linux-libre`.
- `system-profile`: package set from the E2E Guix System profile: `bash`,
  `curl`, `gcc-toolchain`, `guix`, `openssh-sans-x`, and `openssl`.
- `system-build`: full `guix system build` of a controlled E2E operating
  system configuration. This is the preferred benchmark suite for performance
  evidence; `smoke` is primarily a workflow sanity check.

`system-profile` is a reconfigure-like workload: it compares the same
HTTP-only, p2p-only, and p2p-first substitute paths against the packages that
make up the QEMU node's system profile. It does not run `guix system
reconfigure` itself yet; that would require a separate driver for building and
activating a full operating-system generation inside the fetch VM.

`system-build` is the preferred reconfigure precursor benchmark. It builds a
complete operating-system generation inside the seed VM with grafts enabled,
records the closure items that have public substitute narinfo, and seeds those
items through `guix-p2p`.
The generated top-level system output is not timed as a substitute fetch
because it is local to the benchmark and usually has no public narinfo. During
the timed step, each mode invokes the substituter protocol directly for the
recorded public closure paths and restores each NAR to a temporary destination.
This measures the downloadable system payload without relying on `guix build
/gnu/store/...`, which does not force missing store output paths to be
realized.
Each timed system-build fetch uses a fresh temporary restore root because
restored NAR permissions can make a reused destination non-removable between
modes.
The system-build evidence check does not require the generated top-level output
to appear in the fetch-node P2P catalog. For `p2p-only` and `p2p-first`, it
requires seed-node block-serving evidence instead; `http-first` may complete
through HTTP before P2P is used.
The `http` mode uses the same warm `guix-p2p` socket and raw Guix daemon
extension path as the P2P modes, but starts the daemon with HTTP-first policy
and does not require seed-node block-serving evidence. This keeps the
system-build HTTP baseline comparable with P2P modes instead of measuring a
cold direct substitute helper for every public path.
Before each system-build measurement, the harness prepares the mode runner by
realizing the system once and then deleting the top-level output plus closure
items that the seed node already identified as having public substitute
narinfo. Generated/non-public closure items remain local, so each mode is cold
for downloadable substitutes without forcing Guix to rebuild the whole
operating-system closure during the timed run.
The top-level output is deleted before the public closure paths because Guix
will keep referenced paths alive while the generated system output still exists.

For multiple seeders, start and push additional VM nodes, then pass them as a
comma-separated list:

```sh
cargo run -p guix-p2p-e2e -- vm benchmark \
  --suite standard \
  --modes http,p2p-only,p2p-first,http-first \
  --http-conditions normal,dead-primary \
  --seed-nodes Alice,Carol,Dave \
  --fetch-node Bob \
  --http-node Bob \
  --iterations 3
```

The VM benchmark writes:

- `target/guix-p2p-e2e/benchmarks/results.csv`
- `target/guix-p2p-e2e/benchmarks/benchmark-results.md`
- `target/guix-p2p-e2e/benchmarks/logs/*/error.log` for failed runs

The CSV keeps the original result columns and appends phase timings:

- `seed_ms`: seed-node setup for the package/condition.
- `prepare_ms`: realize dependencies and remove only the target output.
- `p2p_start_ms`: start the benchmark `guix-p2p` daemon. For `system-build`,
  this is recorded for warm HTTP and P2P modes.
- `provider_wait_ms`: wait until the target is visible through P2P. This is
  omitted for `system-build`, where availability is checked by fetching the
  recorded public closure paths.
- `daemon_start_ms`: start the extension-enabled raw `guix-daemon`. For
  `system-build`, this is recorded for warm HTTP and P2P modes.
- `import_ms`: run the final package import, or for `system-build`, fetch,
  restore, and verify the recorded public closure NARs in a temporary
  destination tree.
- `total_ms`: total measured mode time.

For `system-build`, the CSV also records `system_build_public_paths`,
`system_build_missing_before`, `system_build_verified`,
`system_build_nar_bytes`, and `system_build_throughput_bps`. The byte count is
summed from the substitute protocol `success sha256:<hash> <bytes>` replies
captured during the timed public-closure fetch.

Failure records keep the full error chain in `results.csv` and write the same
details to `error.log`. The Markdown report keeps the failed-run table concise
and points at the per-run log path.
For `system-build`, failed p2p runs also include the recorded public closure
path head and the public-closure fetch log tail, so the failing substitute path
is visible from the uploaded artifact.

VM benchmark p2p modes do not preload local narinfo metadata into the fetch
node. The fetch-node `guix-p2p` daemon performs the normal remote narinfo
lookup, so `p2p-only` means "no HTTP NAR fallback" rather than "metadata is
already local." For `system-build`, the timed path fetches only public-narinfo
closure substitutes through the configured P2P/HTTP policy and restores them
outside `/gnu/store`. The VM `push-binary` command installs the substitute extension and
loader wrappers for copied Rust binaries so they can find their Guix runtime
libraries inside the guest.
Provider selection now filters candidates through peer reputation and connection
backoff, so stale provider records should be penalized after handshake timeouts
instead of being retried first on later downloads. During an active block
download, request-response failures requeue the affected peer's in-flight blocks
so transient provider connection closes can be retried within the same
substitute attempt. Remaining benchmark work should focus on larger packages,
repeated runs, and HTTP comparison modes before making performance claims.

The older top-level `benchmark` command remains a fast container harness, but
VM benchmarks are the publishable path because each node has its own writable
store and daemon state.

## Container Benchmark

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- benchmark --suite smoke --iterations 1 --transport tcp
```

Defaults:

- suite: `standard`
- standard tiers: small `hello`, medium `git`, large `linux-libre`
- system-profile packages: `bash`, `curl`, `gcc-toolchain`, `guix`,
  `openssh-sans-x`, `openssl`
- modes: `http,p2p-only,p2p-first`
- HTTP conditions: `normal`
- seed counts: `1`
- iterations: `3`
- output directory: `target/guix-p2p-bench/`

Use the smoke suite for quick local checks:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- benchmark --suite smoke --iterations 1 --transport tcp
```

Use the standard suite for publishable evidence across small, medium, and
large packages:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- benchmark \
  --suite standard \
  --modes http,p2p-only,p2p-first,http-first \
  --http-conditions normal,dead-primary \
  --seed-counts 1,3 \
  --iterations 3 \
  --transport tcp \
  --keep-temp
```

Use the system-profile suite for a profile-sized workload based on the E2E VM
system package set:

```sh
guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- benchmark \
  --suite system-profile \
  --modes http,p2p-only,p2p-first \
  --http-conditions normal \
  --seed-counts 1 \
  --iterations 1 \
  --transport tcp
```

Modes:

- `http`: isolated raw `guix-daemon` without the P2P extension.
- `p2p-only`: seed nodes provide the NAR, fetch node is p2p-only,
  `min_providers = 1`.
- `p2p-first`: seed nodes provide the NAR, fetch node tries P2P before HTTP.
- `http-first`: seed nodes provide the NAR, fetch node tries HTTP before P2P.

The local container harness starts isolated raw `guix-daemon` processes. Those
daemon containers expose both the raw daemon closure and the exact `guix`
command closure exported through `GUIX`, so Guix can spawn its built-in
`substitute` command inside the container namespace. Extension-enabled daemon
containers also export the release binary runtime library path before Guix
execs the `guix-p2p` relay.

HTTP conditions:

- `normal`: preferred substitute mirrors first, then the broader fallback list.
- `single-primary`: `https://ci.guix.trop.in` only.
- `single-secondary`: `https://cache-cdn.guix.moe` only.
- `dead-primary`: an unreachable local URL first, then the `normal` mirror list.
- `slow`: reserved for real-network traffic shaping; skipped when shaping is
  unavailable.
- `flaky`: reserved for real-network traffic shaping; skipped when shaping is
  unavailable.

Container HTTP mode builds the resolved benchmark store path, not the package
name, so it measures substitute import for the target item without pulling
unrelated package dependencies into the isolated daemon database.

The preferred mirror order for benchmark defaults is grouped by trust source.
The third-party set comes first for benchmark availability, followed by the
official Guix substitute servers, then additional mirrors:

Third-party preferred:
- `https://ci.guix.trop.in`
- `https://cache-cdn.guix.moe`
- `https://cache-fi.guix.moe`
- `https://guix.bordeaux.inria.fr`
- `https://nonguix-proxy.ditigal.xyz`

Official Guix:
- `https://ci.guix.gnu.org`
- `https://bordeaux.guix.gnu.org`

Additional mirrors:
- `https://cache-sg.guix.moe`
- `https://mirror.yandex.ru/mirrors/guix`
- `https://substitutes.nonguix.org`

Outputs:

- `target/guix-p2p-bench/results.csv`
- `target/guix-p2p-bench/benchmark-results.md`

### GitHub Benchmark Workflow

GitHub benchmarks are manual-only and run the VM benchmark harness from the
mirrored repository. The job builds a base Guix System qcow2 image, boots
Bootstrap, Alice, and Bob, then runs the same private-store VM benchmark used
locally.
The job frees unused hosted-runner toolchains before building the VM image and
stops immediately if image creation fails, so later VM setup errors do not hide
the original Guix image failure.
The GitHub VM guests use 4 GiB of memory. The `system-build` suite can fetch a
large Guix NAR during p2p-only runs, and the current downloader still assembles
that NAR in memory before restore.

To run one:

- Open the GitHub mirror.
- Go to `Actions > Benchmarks`.
- Click `Run workflow`.
- Choose `suite`, `iterations`, and `modes`.
- Download the `guix-p2p-benchmark-*` artifact after the run completes.

The artifact contains:

- `results.csv`
- `benchmark-results.md`
- per-run logs from `target/guix-p2p-bench/logs/`
- VM logs such as `base-image-build.log` when image creation fails

Successful benchmark runs upload a `guix-p2p-benchmark-*` artifact. A separate
Pages workflow listens for successful benchmark runs, downloads that artifact,
and deploys the documentation site with the new report. Normal docs changes can
deploy Pages without waiting for the VM benchmark job.

`index.html` is the project landing page. It focuses on what guix-p2p does,
channel-based setup, configuration, normal usage, and development entrypoints.
`configuration.html` and `deployment.html` render their Markdown source files
with the same static styling as the rest of the Pages site, while keeping raw
Markdown links available for source viewing.
`benchmarks.html` is secondary evidence and renders the latest benchmark report
as HTML tables with links to the raw CSV, raw markdown report, and recent
benchmark workflow runs. Older reports stay attached to their GitHub Actions
runs as artifacts.
Wide benchmark tables scroll horizontally so timing, package, and store path
columns stay readable.
Benchmark report dates are written as UTC datetimes, and the Pages renderer
normalizes older epoch-second reports for display.
The benchmark page also renders dependency-free SVG charts from `results.csv`
when it is available, including elapsed time, phase timing, and system-build
payload throughput. When Pages can download recent successful benchmark
artifacts, it also writes `history.csv` and renders a cross-run trend chart.
Docs-only deploys keep a markdown table fallback.
The static renderer keeps the site dependency-free while adding language labels
and lightweight syntax highlighting for Scheme, shell, and TOML code blocks.
Generated HTML references JavaScript assets with a commit-derived query string
so browsers do not reuse stale renderers after Pages deployments.

Pages deployment runs only when the GitHub mirror supports Pages and the Pages
site is configured for GitHub Actions. If the repository plan or visibility
does not support Pages, benchmark runs still pass and upload the CSV/report
artifact.
Benchmark dispatches do not share a branch-wide concurrency lock, so a stale
run cannot block a later fixed run from starting.

Before running `guix shell`, the workflow writes a systemd drop-in for
`guix-daemon.service` so the runner daemon uses the benchmark mirror list for
all store realizations.

The workflow uses `guix shell -m manifest-ci.scm` with Guix's packaged Rust
toolchain, QEMU, OpenSSH, and a minimal native build environment; it does not
run `guix pull` on benchmark runs.
The VM path avoids the hosted runner's read-only `/gnu/store` by importing
packages inside each node's qcow2 disk.
The workflow embeds the freshly built release binary in the qcow2 image and
uses that image binary inside each VM; it does not push a host-built replacement
binary into the running guests.

Cargo commands export Guix's GCC runtime library directory in
`LD_LIBRARY_PATH` so Rust build scripts can load `libgcc_s.so.1` on hosted CI
runners.

The workflow includes `nss-certs` so `guix shell` exposes a CA bundle for Cargo
to verify crates.io TLS certificates.

Before the first Pages deploy, configure the GitHub mirror's Pages source to
`GitHub Actions` under `Settings > Pages`.

The workflow does not commit generated benchmark output back to either GitHub
or Codeberg. Generated benchmark reports are written under `target/`, not
`docs/`, so they do not create routine repository churn.

The report includes host and Rust summary, tier, package store paths, nar
hashes, nar sizes when observed from dashboard seed data, HTTP condition, seed
count, per-run elapsed time, medians, p95 values, provider counts, P2P
block-serving evidence, HTTP evidence, skipped runs, and failed runs.
For `system-build`, the report and CSV also include public closure item count,
missing-before-fetch count, verified count, fetched NAR bytes, and payload
throughput based on import time.

Per-run temp directories are removed unless `--keep-temp` is passed.

For p2p modes, the container benchmark also runs an explicit relay substitute
restore because shared host-store containers can make exact store-path builds a
no-op. With `--keep-temp`, inspect:

- `$BASE/tmp/<package>-<condition>-<mode>-seed<count>-<iteration>/manual-substitute-output`
- `$BASE/tmp/<package>-<condition>-<mode>-seed<count>-<iteration>/logs/direct-substitute.log`
- `$BASE/tmp/<package>-<condition>-<mode>-seed<count>-<iteration>/logs/seed-1.log`
- `$BASE/tmp/<package>-<condition>-<mode>-seed<count>-<iteration>/logs/node-b.log`

If a kept temp directory contains container-owned files, rerun with a fresh
`--base` rather than deleting the evidence directory.

The smoke and benchmark harnesses require the test container to be able to
write `/gnu/store`, because raw `guix-daemon` imports substituted nars into
the store even with `--max-jobs=0`. If the host exposes `/gnu/store` read-only,
the harness fails at preflight before starting nodes. Some local container
setups can create new store entries but cannot rewrite metadata on existing
host store paths; HTTP-only imports are reported as skipped when substitutes
are found but the isolated daemon cannot make an existing store path writable.
The disposable VM proof is the authoritative full-store-isolation check; the
benchmark harness remains the faster controlled timing tool.

## Future Benchmark Work

The current `hello` result proves the local p2p-only substitute path and block
transfer. It does not prove that P2P is faster than HTTP. Future benchmark work
should compare HTTP and P2P under repeated, controlled scenarios.

Baseline comparison:

- Run `http`, `p2p-only`, and `p2p-first` for the same package set.
- Use a fresh `--base` per run group so kept temp state does not contaminate
  results.
- Use at least three iterations and compare medians, not single runs.
- Record substitute URLs, host Guix revision, transport, package store paths,
  NAR hashes, and NAR sizes.

Package tiers:

- `hello`: small correctness and harness sanity check.
- `git`: medium package with non-trivial closure and transfer size.
- `linux-libre`: large binary package for bandwidth and multi-peer behavior.
- `system-build`: full system generation for reconfigure-like substitute
  behavior and the default CI benchmark target.

Multi-peer P2P comparison:

- Use one fetch node and N seed nodes all seeding the same desired NAR.
- Run seed counts of 1, 3, 5, and 8 for the same package and transport.
- Record provider count, time to first provider, time to first block, total
  restored bytes, elapsed time, and per-seeder block-serving evidence.
- Keep the explicit relay substitute restore for p2p modes; shared host-store
  containers can make exact store-path builds no-op.

Mixed-policy comparison:

- Compare `p2p-first` and `http-first` when P2P providers are available.
- Repeat with `dead-primary`, `slow`, and `flaky` HTTP conditions to measure
  substitute-server failure, latency, and packet-loss behavior. The `slow` and
  `flaky` profiles are recorded as skipped when OS traffic shaping is not
  available.
- Repeat with no providers to measure HTTP fallback latency and confirm the
  fallback path is visible in logs.

Acceptance criteria for publishing a benchmark claim:

- Every p2p run has block-serving evidence in seed-node logs.
- Every restored output exists under the kept run directory.
- HTTP fallback usage is explicitly recorded for `p2p-first` and `http-first`.
- Report median and p95 elapsed time for each tier/package/mode/HTTP
  condition/seed-count group.
