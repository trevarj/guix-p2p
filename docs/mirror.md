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
- installs Guix with the GitHub Guix action on GitHub runners, or with the
  upstream Guix installer fallback when the workflow is executed by Forgejo/act;
- uses `guix shell -m manifest-ci.scm` with Guix's packaged Rust toolchain and
  minimal native build inputs;
- exports Guix's GCC runtime library directory in `LD_LIBRARY_PATH` before
  Cargo commands so build scripts can load `libgcc_s.so.1`;
- runs `cargo fmt --all -- --check`;
- runs `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- runs `cargo test --workspace`.

The workflow is manual-only because Codeberg runs the normal push and pull
request checks. GitHub runner time is reserved for benchmarks and Pages deploys.
If Forgejo/act sees the GitHub workflow file anyway, the Guix install action is
skipped and the shell fallback installs Guix before any `guix shell` step.
The fallback first adds common Guix profile locations to `PATH`, which covers
runner images where Guix is installed but the profile is not sourced.
The fallback runs the installer directly on root runners and uses `sudo` only
when the runner is non-root.
If the runner has `curl` but not `wget`, the fallback provides a temporary
`wget` shim for the Guix installer.
The fallback treats a nonzero installer exit as recoverable when the `guix`
binary was installed, then starts `guix-daemon --disable-chroot` manually for
the later `guix shell` steps.

## Codeberg CI

Codeberg runs a lightweight availability check from `.forgejo/workflows/ci.yml`
on the repository runner labeled `codeberg-small`.
When that runner cannot provide or install Guix, the Forgejo workflow exits the
Rust check steps successfully after printing a skip message. This keeps
Codeberg push status from failing on runner provisioning. The GitHub CI and
benchmark workflows remain strict because they run on runners where Guix is
installed by workflow setup.

The workflow:

- runs on pushes to `master`, pull requests, and manual dispatches;
- checks out the repository with the Forgejo checkout action;
- installs Guix with the upstream installer when `guix` is not already present
  on the runner;
- verifies the runner's Guix installation with `guix --version` and
  `guix describe`;
- prints skip messages for `cargo fmt`, `cargo clippy`, and `cargo test`.

The job uses `runs-on: codeberg-small`, matching the Codeberg hosted runner
label.

## GitHub Benchmarks

The manual benchmark workflow is `.github/workflows/benchmarks.yml`.

To run it:

- Open the GitHub mirror.
- Go to `Actions > Benchmarks`.
- Click `Run workflow`.
- Choose `suite`, `iterations`, and `modes`.
- Download the `guix-p2p-benchmark-*` artifact from the completed run.

The workflow runs the VM benchmark harness and uploads
`target/guix-p2p-bench/results.csv` plus
`target/guix-p2p-bench/benchmark-results.md`. It does not commit generated
benchmark output back to the repository.

Before running `guix shell`, the workflow restarts `guix-daemon.service` with a
systemd drop-in that sets the benchmark substitute URL list. This keeps Guix
package realization on the GitHub runner from depending only on the default
substitute servers.

The workflow uses `guix shell -m manifest-ci.scm` with Guix's packaged Rust
toolchain, QEMU, OpenSSH, and a minimal native build environment; it does not
run `guix pull` on benchmark runs.

GitHub runs the VM benchmark harness under `sudo`, builds a base Guix System
qcow2 image, boots Bootstrap, Alice, and Bob with QEMU, and runs the benchmark
inside those private-store nodes. This avoids the hosted runner's read-only
`/gnu/store` and keeps skipped container reports from being published as
successful benchmark evidence.

Cargo commands export Guix's GCC runtime library directory in
`LD_LIBRARY_PATH` so Rust build scripts can load `libgcc_s.so.1` on hosted CI
runners.

The workflow includes `nss-certs` so `guix shell` exposes a CA bundle for Cargo
to verify crates.io TLS certificates.

The separate GitHub Pages workflow deploys the documentation site on docs
pushes and after successful benchmark runs. Before the first deploy, set the
GitHub mirror's Pages source to `GitHub Actions` under `Settings > Pages`.
Pages must also be supported by the repository's GitHub plan and visibility.
When Pages is unavailable, the benchmark workflow still succeeds and keeps the
CSV/report as a run artifact.

The Pages site publishes a user-facing project home at `index.html` and a
rendered benchmark report at `benchmarks.html`. The home page focuses on
project purpose, channel-based setup, configuration, normal usage, and
development entrypoints. The benchmark page renders the latest markdown report
as HTML tables and links to the raw CSV, raw markdown report, and recent
benchmark workflow runs. Older CSV and markdown reports remain attached to
their corresponding GitHub Actions runs as artifacts.
The site generator copies the `docs/assets/guix-p2p-wordmark.svg` wordmark into
the Pages artifact and uses it in the header and hero. Benchmark dates are
shown as UTC datetimes; older epoch-second reports are normalized in the
browser renderer.
The benchmark page renders SVG charts in the browser from `results.csv`, and
falls back to the markdown report tables when a docs-only Pages deploy does not
include a fresh CSV artifact.
The static renderer also provides dependency-free code block language labels
and lightweight highlighting for Scheme, shell, and TOML examples.

## References

- Forgejo repository mirroring: <https://forgejo.org/docs/next/user/repo-mirror/>
- GitHub repository duplication: <https://docs.github.com/en/repositories/creating-and-managing-repositories/duplicating-a-repository>
- Guix GitHub Action: <https://github.com/PromyLOPh/guix-install-action>
