# Codeberg To GitHub Mirror

Codeberg is the canonical repository. GitHub is a CI mirror only.

## GitHub Repository

Create an empty repository:

- `github.com/trevarj/guix-p2p`
- Do not initialize it with a README, license, or `.gitignore`.

Create a fine-grained GitHub personal access token:

- Repository access: only `trevarj/guix-p2p`
- Permissions:
  - `Contents: Read and write`
  - `Workflows: Read and write`

`Workflows` is required because Codeberg pushes `.github/workflows/*`.

## Codeberg Push Mirror

In `trevarj/guix-p2p` on Codeberg:

- Open `Settings > Repository > Mirror Settings`.
- Add a push mirror:
  - Remote URL: `https://github.com/trevarj/guix-p2p.git`
  - Username: GitHub username
  - Password: GitHub token
  - Branch filter: leave blank to mirror all branches and tags
  - Enable `Sync when new commits are pushed`
- Click `Synchronize Now`.

Verify that GitHub receives:

- `master`
- tags
- full commit history

Leave the GitHub repository empty until the first mirror sync, or make sure it
already has the same history. Codeberg push mirroring may force-push when all
branches and tags are mirrored.

## Operating Rules

- Treat GitHub as read-only for humans.
- Do not merge GitHub pull requests directly.
- Port any GitHub pull request changes to Codeberg manually.
- Do not configure GitHub-to-Codeberg mirroring.
- Keep generated benchmark reports out of automated CI commits.

## GitHub CI

GitHub Actions runs from mirrored workflow files. The active workflow is
`.github/workflows/ci.yml`.

The workflow:

- runs only when manually dispatched;
- installs Guix on the GitHub runner;
- uses `guix shell -m manifest-ci.scm` with Guix's packaged Rust toolchain and
  minimal native build inputs;
- exports Guix's GCC runtime library directory in `LD_LIBRARY_PATH` before
  Cargo commands so build scripts can load `libgcc_s.so.1`;
- runs `cargo fmt --all -- --check`;
- runs `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- runs `cargo test --workspace`.

The workflow is manual-only because Codeberg runs the normal push and pull
request checks. GitHub runner time is reserved for benchmarks and Pages deploys.

## Codeberg CI

Codeberg runs the normal CI checks from `.forgejo/workflows/ci.yml` on the
repository runner labeled `codeberg-small`.

The workflow:

- runs on pushes to `master`, pull requests, and manual dispatches;
- checks out the repository with the Forgejo checkout action;
- verifies the runner's Guix installation with `guix --version` and
  `guix describe`;
- uses `guix shell -m manifest-ci.scm` with Guix's packaged Rust toolchain and
  minimal native build inputs;
- exports Guix's GCC runtime library directory in `LD_LIBRARY_PATH` before
  Cargo commands so build scripts can load `libgcc_s.so.1`;
- runs `cargo fmt --all -- --check`;
- runs `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- runs `cargo test --workspace`.

The job uses `runs-on: codeberg-small`, matching the Codeberg hosted runner
label.

## GitHub Benchmarks

The manual benchmark workflow is `.github/workflows/benchmarks.yml`.

To run it:

- Open the GitHub mirror.
- Go to `Actions > Benchmarks`.
- Click `Run workflow`.
- Choose `suite`, `iterations`, and `transport`.
- Download the `guix-p2p-benchmark-*` artifact from the completed run.

The workflow runs the container benchmark harness and uploads
`target/guix-p2p-bench/results.csv` plus `docs/benchmark-results.md`. It does
not commit generated benchmark output back to the repository.

Before running `guix shell`, the workflow restarts `guix-daemon.service` with a
systemd drop-in that sets the benchmark substitute URL list. This keeps Guix
package realization on the GitHub runner from depending only on the default
substitute servers.

The workflow uses `guix shell -m manifest-ci.scm` with Guix's packaged Rust
toolchain and a minimal native build environment; it does not run `guix pull`
on benchmark runs.
The CI manifest also includes the Guix CLI so the benchmark harness can spawn
nested Guix container environments without relying on the runner's ambient
PATH.

GitHub runs the benchmark harness under `sudo` because the container benchmark
uses nested `guix shell -CN` environments that need mount privileges for a
writable `/gnu/store`.
Hosted GitHub runners cannot reliably share the checked-out repository into
nested Guix containers, so the workflow sets `GUIX_P2P_E2E_NO_GUIX_SHELL=1`
and lets the disposable runner provide the isolation boundary.
If hosted runners expose `/gnu/store` read-only, the workflow sets
`GUIX_P2P_E2E_ALLOW_READ_ONLY_STORE=1` so the harness publishes an explicit
skipped report and still updates artifacts/Pages. Full benchmark evidence still
requires local containers with writable store isolation or the VM benchmark
path.

Cargo commands export Guix's GCC runtime library directory in
`LD_LIBRARY_PATH` so Rust build scripts can load `libgcc_s.so.1` on hosted CI
runners.

The workflow includes `nss-certs` so `guix shell` exposes a CA bundle for Cargo
to verify crates.io TLS certificates.

Successful benchmark runs also deploy a GitHub Pages site. Before the first
deploy, set the GitHub mirror's Pages source to `GitHub Actions` under
`Settings > Pages`.
The workflow passes `enablement: true` to `actions/configure-pages` so the
mirror can create or repair that Pages Actions source configuration during
deploy when the token has `pages: write`.

The Pages site shows the latest generated report and links to recent benchmark
workflow runs. Older CSV and markdown reports remain attached to their
corresponding GitHub Actions runs as artifacts.

## References

- Forgejo repository mirroring: <https://forgejo.org/docs/next/user/repo-mirror/>
- GitHub repository duplication: <https://docs.github.com/en/repositories/creating-and-managing-repositories/duplicating-a-repository>
- Guix GitHub Action: <https://github.com/PromyLOPh/guix-install-action>
