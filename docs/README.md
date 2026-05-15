# Documentation

Active docs are the current source of truth for operating, validating, and
developing `guix-p2p`.

| File | Purpose |
|------|---------|
| [architecture.md](architecture.md) | Architecture, crate map, data flow, dashboard surfaces |
| [configuration.md](configuration.md) | TOML keys, defaults, CLI overrides |
| [deployment.md](deployment.md) | Daemon, relay, wrapper, and isolated Guix flow |
| [scripts.md](scripts.md) | Script inventory and Rust migration status |
| [bootstrap-node.md](bootstrap-node.md) | Shepherd-first bootstrap node operation |
| [e2e.md](e2e.md) | Strict two-node VM proof |
| [benchmarks.md](benchmarks.md) | Smoke tests, benchmark harness, future benchmark method |
| [benchmark-results.md](benchmark-results.md) | Latest generated benchmark report |
| [mirror.md](mirror.md) | Codeberg-to-GitHub mirror and GitHub CI setup |
| [dht-protocol.md](dht-protocol.md) | Kademlia DHT design |
| [swarm-protocol.md](swarm-protocol.md) | Block exchange wire protocol |
| [roadmap.md](roadmap.md) | Current remaining work |
| [future-ideas.md](future-ideas.md) | Deferred implementation ideas |

Historical plans and implementation prompts live in [archive/](archive/).
Do not treat archived checklists as current status without checking the active
docs and code.
