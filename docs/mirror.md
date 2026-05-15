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

- runs on pushes to `master`, version tags, and manual dispatches;
- installs Guix on the GitHub runner;
- shallow-clones the `guix-rustup` channel and passes it with `guix shell -L`
  so the `rustup` module used by `manifest.scm` is available;
- uses `guix shell -m manifest.scm`, including the pinned nightly Rust toolchain;
- exports Guix's GCC runtime library directory in `LD_LIBRARY_PATH` before
  Cargo commands so build scripts can load `libgcc_s.so.1`;
- runs `cargo fmt --all -- --check`;
- runs `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- runs `cargo test --workspace`.

The workflow deliberately does not auto-commit generated benchmark output.

## Codeberg CI

Codeberg runs the normal CI checks from `.forgejo/workflows/ci.yml` on the
repository runner labeled `codeberg-small`.

The workflow:

- runs on pushes to `master`, pull requests, and manual dispatches;
- checks out the repository with the Forgejo checkout action;
- verifies the runner's Guix installation with `guix --version` and
  `guix describe`;
- shallow-clones the `guix-rustup` channel and passes it with `guix shell -L`
  so the `rustup` module used by `manifest.scm` is available;
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

The workflow shallow-clones the `guix-rustup` channel and passes
`.cache/guix-rustup/guix` with `guix shell -L`; this provides the `rustup`
module used for the pinned nightly Rust toolchain without running `guix pull`
on every benchmark run.

Cargo commands export Guix's GCC runtime library directory in
`LD_LIBRARY_PATH` so Rust build scripts can load `libgcc_s.so.1` on hosted CI
runners.

The workflow includes `nss-certs` so `guix shell` exposes a CA bundle for Cargo
to verify crates.io TLS certificates.

Successful benchmark runs also deploy a GitHub Pages site. Before the first
deploy, set the GitHub mirror's Pages source to `GitHub Actions` under
`Settings > Pages`.

The Pages site shows the latest generated report and links to recent benchmark
workflow runs. Older CSV and markdown reports remain attached to their
corresponding GitHub Actions runs as artifacts.

## References

- Forgejo repository mirroring: <https://forgejo.org/docs/next/user/repo-mirror/>
- GitHub repository duplication: <https://docs.github.com/en/repositories/creating-and-managing-repositories/duplicating-a-repository>
- Guix GitHub Action: <https://github.com/PromyLOPh/guix-install-action>
