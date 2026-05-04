# guix-p2p-substitute

P2P substitute distribution for GNU Guix using libp2p Kademlia DHT + custom
block-swarm protocol.

## Status

Phase 1 planning. No code yet.

## Quick Start (Future)

```sh
# Install
guix pull --channel=https://example.org/guix-p2p-channel.git
guix install guix-p2p-substitute

# Run as daemon proxy (background DHT node + HTTP substitute server)
guix-p2p-substitute --daemon --listen 0.0.0.0:8080 &

# Use in guix
GUIX_USE_P2P=yes guix build hello
```

## Documentation

- [architecture.md](docs/architecture.md) — complete architecture
- [implementation-plan.md](docs/implementation-plan.md) — phased roadmap
- [dht-protocol.md](docs/dht-protocol.md) — Kademlia DHT design
- [swarm-protocol.md](docs/swarm-protocol.md) — block exchange protocol

## License

GPL-3.0-or-later
