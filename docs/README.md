# Documentation

Active docs are the current source of truth for operating, validating, and
developing `guix-p2p`.

| File | Purpose |
|------|---------|
| [architecture.md](architecture.md) | Architecture, crate map, data flow, dashboard surfaces |
| [configuration.md](configuration.md) | TOML keys, defaults, CLI overrides |
| [tester-quickstart.md](tester-quickstart.md) | First-run tester flow, dashboard basics, and failure report shape |
| [tester-issue-template.md](tester-issue-template.md) | Copyable issue template for tester failures |
| [connectivity.md](connectivity.md) | Bootstrap, NAT, firewall, and dashboard connectivity signals |
| [troubleshooting.md](troubleshooting.md) | Common tester failures and what to check |
| [deployment.md](deployment.md) | Guix channel setup, daemon, relay, extension, and isolated Guix flow |
| [scripts.md](scripts.md) | Script inventory and Rust migration status |
| [bootstrap-node.md](bootstrap-node.md) | Shepherd-first bootstrap node operation |
| [e2e.md](e2e.md) | Strict two-node VM proof |
| [benchmarks.md](benchmarks.md) | Smoke tests, benchmark harness, future benchmark method |
| [relay-overhead.md](relay-overhead.md) | Plan for measuring and reducing substitute relay startup overhead |
| [mirror.md](mirror.md) | Codeberg-to-GitHub mirror and GitHub CI setup |
| [dht-protocol.md](dht-protocol.md) | Kademlia DHT design |
| [swarm-protocol.md](swarm-protocol.md) | Block exchange wire protocol |
| [contributing.md](contributing.md) | Contribution and AI-assisted work policy |
| [roadmap.md](roadmap.md) | Current remaining work |
| [future-ideas.md](future-ideas.md) | Deferred implementation ideas |

Historical plans and implementation prompts live in [archive/](archive/).
Do not treat archived checklists as current status without checking the active
docs and code.
