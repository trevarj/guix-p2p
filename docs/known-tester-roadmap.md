# Known Tester Roadmap

This tracks the work needed to make `guix-p2p` easy to share with known
testers and demo clearly. The core P2P substitute path is already proven by
the strict VM E2E flow, but the project still needs a better setup story,
dashboard-guided seeding, and clearer demo/readiness boundaries before a
broader public network.

## Current Readiness

- [x] P2P substitute path works through the Guix substituter protocol.
- [x] Strict VM proof validates separate stores and real `guix build` import.
- [x] Nodes can use a manually shared bootstrap multiaddr.
- [x] Explicit `--seed` and `seed_paths` can seed store paths at daemon startup.
- [x] Successful downloads are cached and re-announced for re-seeding.
- [x] Setup docs are complete enough for known testers.
- [x] Dashboard can seed packages interactively.
- [x] Dashboard clearly shows the live P2P benefit path.
- [x] Public network readiness is not claimed yet.

## Setup Docs

- [x] Add a README `Setup` section for known testers.
- [x] Document the current install path: `guix shell -m manifest.scm` plus
  `cargo build --release`.
- [x] Document how to configure `bootstrap_peers` from a manually shared
  bootstrap multiaddr.
- [x] Document how to run `guix-p2p --daemon --dashboard` with cache and socket
  paths.
- [x] Document the current `scripts/guix-wrapper.sh` substitute flow.
- [x] Link strict validation to `docs/e2e.md`.
- [x] State known public-network gaps:
  - stable bootstrap infrastructure
  - Guix packaging/channel story
  - authenticated remote dashboard mutation
  - clearer operational support expectations

## Dashboard Package Seeding

- [x] Add `GET /api/packages`.
- [x] Source packages from:
  - `/run/current-system/profile`
  - `/run/current-system/kernel`
  - `$HOME/.guix-home/profile`
- [x] Use `guix package --list-installed --profile=<profile>` without a regex.
  The Guix manual documents the regexp as optional.
- [x] Return package entries with:
  - `source`
  - `name`
  - `version`
  - `output`
  - `store_path`
  - `seeded`
- [x] Add a fuzzy-search package picker to the dashboard.
- [x] Search across package name, version, and store path.
- [x] Show source, package name, version, output, store path, and seed state.
- [x] Add a per-row `Seed` action.

## Dashboard Seed Mutation

- [x] Add `POST /api/seeds` with `{ "store_path": "/gnu/store/..." }`.
- [x] Allow mutation only when the dashboard bind address is loopback.
- [x] Validate that the requested path starts with `/gnu/store/`.
- [x] Validate that the requested path exists.
- [x] Seed immediately by exporting, caching, and announcing the NAR.
- [x] Emit `SeedAdded` after successful seeding.
- [x] Persist successful selections to user config `seed_paths`.
- [x] Write only to:
  - `$XDG_CONFIG_HOME/guix-p2p/config.toml`
  - `~/.config/guix-p2p/config.toml` when `XDG_CONFIG_HOME` is unset
- [x] Deduplicate persisted `seed_paths`.
- [x] Add `toml_edit` to preserve existing user config formatting/comments.
- [x] Extend dashboard state with a `SwarmCommand` sender so new seeds can
  trigger `StartProviding`.
- [x] Extend NAR seed metadata to retain optional `store_path` for dashboard
  display and `seeded` matching.

## Dashboard Stop Seeding

- [x] Add a dashboard action to stop seeding an active package/NAR.
- [x] Add an API mutation for seed removal.
- [x] Remove the matching store path from persisted `seed_paths`.
- [x] Stop serving the NAR locally or mark it inactive in the serving index.
- [x] Emit `SeedRemoved` after successful removal.
- [x] Update the package picker action from `Seed` to `Stop seeding` when
  `seeded` is true.
- [x] Document DHT provider withdrawal behavior or expiry limitations.

## Dashboard Layout

- [x] Restructure the dashboard around the known-tester workflow:
  package picker, active seeds, then network diagnostics.
- [x] Make packages and seeds the primary panels.
- [x] Move peers, catalog, and builds into secondary diagnostic panels.
- [x] Keep events full-width but less dominant by default.

## Dashboard Visual Guide

- [x] Add a live transfer path panel.
- [x] Show the latest request through these stages:
  - package observed
  - narinfo trusted
  - providers found
  - blocks received or served
  - verified/imported
  - re-seeded locally
- [x] Keep the panel operational and event-driven; avoid marketing copy.
- [x] Preserve existing dashboard panels for peers, catalog, builds, and seeds.

## Tests And Acceptance

- [x] Unit test `guix package --list-installed` tab-separated parsing.
- [x] Unit test missing system/home profiles are skipped cleanly.
- [x] Unit test `seed_paths` config persistence and deduplication.
- [x] Unit test non-`/gnu/store` dashboard seed requests are rejected.
- [x] Unit test localhost-only mutation enforcement.
- [x] API test `GET /api/packages`.
- [ ] API test `POST /api/seeds` seeds, announces, emits `SeedAdded`, and
  persists config.
- [x] API test `DELETE /api/seeds/{hash}` removes cached seeds without
  store-path metadata.
- [ ] Run `cargo fmt`.
- [ ] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Run `cargo test`.
- [ ] Run the strict VM proof from `docs/e2e.md`.

## Decisions Locked

- Audience: known testers.
- Install path: Guix shell plus Cargo build.
- Bootstrap: manual multiaddr shared with testers.
- Dashboard mutation safety: localhost-only.
- Package picker scope: system and Guix Home profiles.
- Seed behavior: seed the running daemon immediately and persist to user config.
