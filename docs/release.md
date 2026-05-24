# Release Process

Current supported install path:

- authenticated Guix channel
- source build from the channel checkout
- Guix System service configuration

Prebuilt release binaries are not supported yet. The E2E harness copies local
Rust binaries into disposable VMs for testing, but that is not a user-facing
distribution channel.

## Versioning

Use SemVer for release-visible code, packaging, protocol, CLI, config, service,
or behavior changes.

Before committing a release-visible change:

```sh
scripts/bump-version.sh patch
```

Use `minor` for backward-compatible features and `major` for incompatible
CLI/config/protocol/API changes. Docs-only, test-only, CI-only, comment-only,
and formatting-only changes do not require a version bump.

The version bump script keeps these files aligned:

- `Cargo.toml`
- `Cargo.lock`
- `channel/guix-p2p/packages.scm`

## Tagging

Only tag a release when the current `Cargo.toml` version is intended to be the
published version.

Checklist before tagging:

1. `guix-p2p --version` prints the crate version and Git commit.
2. `scripts/bump-version.sh` has already aligned Cargo and channel package
   versions when needed.
3. Known-tester setup docs match the service defaults.
4. Bootstrap node operation docs match the live bootstrap node.
5. CI smoke checks pass on the GitHub mirror.
6. The strict VM channel proof passes for the release candidate.
7. Pages deploys from the same Git commit as the tag.

Tag format:

```sh
git tag -s vX.Y.Z -m "guix-p2p X.Y.Z"
```

Then push the signed tag to Codeberg and confirm the GitHub mirror receives it.

## Source Snapshots

The current channel package builds from the channel checkout so authenticated
Guix channel users get the checked-out source directly.

For a broader public release, prefer signed Git tags as the human release
boundary. Keep the channel checkout flow for normal users unless packaging
evidence shows that tagged source snapshots make Guix evaluation or
reproducibility materially clearer.

## Binary Releases

Do not publish binaries until there is a documented runtime and trust story:

- target platforms and libc/runtime assumptions
- how the Guix substitute extension is installed
- how the binary embeds or reports the Git commit
- how users verify the binary provenance
- how bootstrap operators upgrade without changing PeerId

Until then, source builds through the authenticated Guix channel are the
documented release path.
