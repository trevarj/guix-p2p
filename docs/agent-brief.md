# Agent Brief

Use this page first when an automated coding agent needs to modify `guix-p2p`.
It is intentionally short and points to deeper docs only when needed.

## Current Shape

`guix-p2p` is a Rust daemon plus Guix integration:

- The daemon keeps a warm libp2p swarm, a NAR cache, peer reputation, provider
  discovery, and the dashboard.
- Guix substitute calls reach the daemon through the Guix substitute extension
  and Unix socket relay path.
- NAR bytes from peers are verified against trusted Guix narinfo hashes before
  import. Peers are never trusted for integrity.
- HTTP substitutes remain available according to policy. The known-tester
  default is `http-first` while the public P2P network is sparse.

## Most Useful Files

| Task | Start Here |
|------|------------|
| Substitute flow, relay, policy | `src/daemon.rs`, `src/relay.rs`, `docs/deployment.md` |
| libp2p behavior, DHT, peers | `src/behaviour.rs`, `src/dht.rs`, `src/connection.rs`, `docs/dht-protocol.md` |
| Block transfer | `src/swarm/`, `docs/swarm-protocol.md` |
| HTTP/narinfo verification | `src/http_client.rs`, `src/narinfo.rs` |
| Cache and seed metadata | `src/nar_store.rs`, `docs/configuration.md` |
| Dashboard/API | `src/dashboard.rs`, `src/dashboard.html`, `docs/architecture.md` |
| Guix service/channel | `channel/`, `docs/tester-quickstart.md`, `docs/deployment.md` |
| Tester failure triage | `docs/troubleshooting.md`, `docs/tester-issue-template.md` |
| E2E and benchmarks | `e2e/`, `docs/e2e.md`, `docs/benchmarks.md` |
| Website | `scripts/build-pages-site.sh`, `site/` |

## Task Routing

Before editing:

1. Read `AGENTS.md`.
2. Read the relevant row from the table above.
3. Search the code with `rg` for the CLI flag, config key, event name, or type
   you intend to change.
4. Prefer existing patterns over new abstractions.

Use `docs/README.md` to classify docs before relying on them. Files listed
under "Plans, Records, And Deferred Work" are context, not proof that the code
still needs that exact plan.

When changing behavior:

- Update `docs/architecture.md` if data flow, dashboard behavior, or config
  meaning changes.
- Update `docs/configuration.md` for config, CLI, service, or env var changes.
- Update `docs/dht-protocol.md` or `docs/swarm-protocol.md` for protocol
  changes.
- Update `docs/tester-quickstart.md` when tester commands or expectations
  change.
- Update `scripts/build-pages-site.sh` and the generated `site/` output when
  human-facing website pages change.

## Invariants

- Do not bypass Guix narinfo/hash verification for peer data.
- Do not make non-loopback dashboard mutation safe by assumption; document and
  test any expansion of that surface.
- Do not advertise private, loopback, or unspecified addresses as public peer
  addresses.
- Do not depend on a user shell profile for the system service path.
- Keep `guix-p2p --doctor` and dashboard diagnostics aligned.
- Keep committed generated docs/site files in sync with their source docs.

## Verification

For Rust code changes, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -j 1
```

For docs or site changes, run:

```sh
scripts/build-pages-site.sh
git diff --check
```

If `cargo test` fails inside a sandbox with local socket permission errors,
rerun the same command outside the sandbox before treating it as a code failure.

## Human-Facing Docs

Keep human docs concise:

- `README.md`: quick project overview and minimum install path.
- `docs/tester-quickstart.md`: step-by-step tester playbook with examples.
- `docs/configuration.md`: reference table plus short examples.
- `docs/deployment.md`: deeper service and daemon operation.
- `docs/troubleshooting.md`: symptom-first fixes.

Avoid duplicating long explanations across these files. Link to the reference
doc instead.
