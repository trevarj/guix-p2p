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
- runs `cargo fmt --all -- --check`;
- runs `cargo clippy --all-targets --all-features -- -D warnings`;
- runs `cargo test`.

The workflow deliberately does not auto-commit generated benchmark output.

## References

- Forgejo repository mirroring: <https://forgejo.org/docs/next/user/repo-mirror/>
- GitHub repository duplication: <https://docs.github.com/en/repositories/creating-and-managing-repositories/duplicating-a-repository>
- Guix GitHub Action: <https://github.com/PromyLOPh/guix-install-action>
