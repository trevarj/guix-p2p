# AGENTS.md — guix-p2p Agent Rules

## Documentation

After every task — feature, bug fix, refactor, or protocol change:
- Update the relevant doc in `docs/` before marking the task complete.
- If architecture or data flow changed, update `docs/architecture.md`.
- If protocol messages or wire format changed, update `docs/swarm-protocol.md` or `docs/dht-protocol.md`.
- If scope, phases, or timeline changed, update `docs/implementation-plan.md`.
- If a new config option, CLI flag, or env var was added, update `docs/architecture.md` (crate deps / config sections).
- Docs must reflect the code as it exists now, not aspirations.
- Make a commit using the standards listed below.

## Git Commits

- Conventional Commits: `type: description`
- Types: `feat`, `fix`, `refactor`, `chore`, `docs`, `test`, `build`, `ci`, `perf`, `style`
- Keep messages short — single line when possible.
- Use bullet points in body if more detail needed.
- Never commit secrets, tokens, or credentials.

## Rust Conventions

### Formatting
Always run before committing:
```
cargo fmt
```

### Linting
Always run before committing and fix all warnings:
```
cargo clippy --all-targets --all-features -- -D warnings
```

### Style
- Follow standard Rust 2024 idioms.
- Use `tracing` for logging (not `println!`, not `log`).
- Prefer `anyhow` for application-level errors, `thiserror` for library-level errors.
- Keep `unwrap()` and `expect()` out of production code paths — use proper error handling.
- Public API types get doc comments (`///`).

### Testing
Always run before marking a task complete:
```
cargo test
```

Write tests alongside code:
- Unit tests in the same file (`#[cfg(test)] mod tests { ... }`).
- Integration tests in `tests/`.
- Use proptest or quickcheck for protocol parsing when appropriate.

## Build

This project uses crates.io for dependency resolution during development.
The plan is to package it for Guix later.

Common commands:
```
cargo build              # debug build
cargo build --release    # release build
cargo test               # run all tests
cargo fmt                # format code
cargo clippy --all-targets --all-features -- -D warnings  # lint
cargo run -- --help      # run the binary
```

## Before Marking a Task Complete

Run in order:
1. `cargo fmt`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test`
4. Update relevant docs in `docs/`
5. Confirm all changes are reflected in docs

## Project Structure

```
guix-p2p-substitute/
├── Cargo.toml
├── src/
│   ├── main.rs               # CLI, swarm task, daemon task, bidirectional channels
│   ├── channel.rs            # SwarmCommand / SwarmNotification enums
│   ├── daemon.rs             # stdin parser, fd 4 reply writer, swarm substitute pipeline
│   ├── behaviour.rs          # libp2p NetworkBehaviour (kad + block_exchange + mdns + identify)
│   ├── dht.rs                # Kad wrapper, handle_kad_event → notifications, get_providers
│   ├── swarm/
│   │   ├── mod.rs
│   │   ├── block.rs          # Block split/join, SHA-256, bitfields, BlockInfo
│   │   ├── codec.rs          # Request/response codec (cbor BlockRequest/BlockResponse)
│   │   └── downloader.rs     # ActiveDownload state machine, peer pool
│   ├── http_client.rs        # Narinfo fetch (HTTP only), signature verification, cache
│   ├── narinfo.rs            # Narinfo parser, ACL loader, Ed25519 verifier, NarinfoCache
│   ├── identity.rs           # Ed25519 keypair gen/persistence
│   └── config.rs             # Config struct (block_size, timeouts, acl_path, substitute_urls, etc.)
└── tests/
    ├── integration.rs
    └── block.rs
```

## Dependencies

Do not add new dependencies without justification. If a dependency is needed:
- Explain why in the commit message.
- Update `docs/architecture.md` crate table.
- Prefer crates already in the tree.
