# Substitute Extension Integration Record

## Summary

`guix-p2p` uses Guix's command extension mechanism to intercept `guix substitute` without
wrapping the `guix` binary. Guix searches `GUIX_EXTENSIONS_PATH` before its
built-in `(guix scripts substitute)` module, and `guix-daemon` can receive that
environment through `guix-configuration`.

This is the preferred integration. The Rust `guix-p2p-wrapper` remains
installed as a compatibility fallback while the channel service helper configures
the extension path for normal Guix System usage.

## Implemented Shape

- The repository is a Guix channel with modules under `channel/guix-p2p/`.
- `guix.scm` is a compatibility entrypoint for local `guix build -f guix.scm`
  and `guix shell -f guix.scm` workflows.
- The package installs a Guile extension module `(guix extensions substitute)` exporting
  `guix-substitute`.
- The extension handles only `--query` and `--substitute`:
  - if the `guix-p2p` relay socket exists, connect to it directly from Scheme,
    forward stdin protocol lines, write reply lines to fd 4, and restore
    daemon `nar:` chunks to the substitute destination;
  - otherwise, delegate to built-in `(guix scripts substitute)`.
- The package installs the extension under
  `share/guix/extensions/substitute.scm` and exposes `$GUIX_EXTENSIONS_PATH` as
  a native search path.
- `(guix-p2p services)` exports `guix-p2p-enable-guix-daemon-extension`, which
  prepends the package extension directory to any existing
  `GUIX_EXTENSIONS_PATH` and sets `GUIX_P2P_SOCKET`. It still accepts the
  legacy `#:guix-p2p-bin` keyword so existing system configs keep evaluating,
  but the extension no longer uses `GUIX_P2P_BIN`.
- `(guix-p2p services)` still exports `guix-p2p-enable-guix-daemon-wrapper` for
  compatibility with older setups.

Normal user commands remain unchanged:

```sh
guix build hello
```

No wrapper-prefixed `guix build` commands, and no quick-start env-var ceremony.

## E2E Proof

`cargo run -p guix-p2p-e2e -- vm channel-proof` is the deployment proof. It
copies the local channel modules into the fetch VM, imports `(guix-p2p
services)`, calls `guix-p2p-enable-guix-daemon-extension`, starts a raw
`guix-daemon` with the resulting environment, and proves that a normal
`guix build hello` imports through the P2P substitute path.

## Upstream Track

The extension path works without Guix daemon changes. A cleaner future Guix
patch would add a daemon-owned hook:

- Add `guix-daemon --substituter-program=FILE`.
- When unset, preserve current behavior: execute `guix substitute --query` /
  `--substitute`.
- When set, execute `FILE --query` / `FILE --substitute` directly.
- Do not expose this through client `set-build-options`; it must remain
  daemon/operator controlled.
- Update daemon docs, `guix-configuration`, and daemon tests.

## Test Plan

- Rust:
  - `cargo fmt`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test`
- Guix package/service:
  - `guix build -f guix.scm`
  - verify the package output contains `share/guix/extensions/substitute.scm`;
  - verify the service helper configures `GUIX_EXTENSIONS_PATH` instead of
    `GUIX`.
- Deployment:
  - `cargo run -p guix-p2p-e2e -- vm channel-proof`
- CI benchmark remains the place for smoke benchmarks; no local benchmark runs.

## Assumptions

- Recommended flow uses the extension, not the `GUIX` wrapper.
- `guix-p2p` remains responsible for p2p-vs-http policy; no HTTP/P2P racing is
  introduced.
- Default socket stays `/var/cache/guix-p2p/guix-p2p.sock` for system service
  use.
- Rust relay mode, wrapper binary, and wrapper script stay temporarily for
  compatibility, rollback, and debugging.
