# DHT Protocol Design

## Overview

libp2p-kad (Kademlia with S/Kademlia modifications) provides a SHA-256-keyed
DHT for finding which peers have a given store item (nar archive).

Key space: 256-bit SHA-256
Key: nar-SHA-256 (32 bytes, from narinfo `NarHash` field)
Record type: Provider records (PeerId → "I have this nar")

## Operations

### Put (announce availability)

```rust
swarm.behaviour_mut().kad.start_providing(nar_hash).expect("key must be valid");
```

This registers the local PeerId as a provider for `nar_hash`. libp2p-kad
handles:
- Persisting the record in the local MemoryStore
- Republishing to the k=20 closest nodes
- TTL management and periodic refresh

### Get (discover providers)

```rust
let providers = swarm.behaviour_mut().kad.get_providers(nar_hash);
// QueryResult::GetProviders { key, providers, closest_peers }
```

Returns:
- `providers`: Vec<PeerId> of peers that have called `start_providing()` for this key
- `closest_peers`: routing table fallback if no providers found

### Bootstrap

```rust
for addr in &config.bootstrap_peers {
    kad.add_address(peer_id, address_without_p2p_suffix);
    swarm.dial(addr).await;
    // After k-bucket population, periodic bootstrap via iterative FIND_NODE
}
```

Bootstraps:
1. Connect to built-in project nodes, configured community/private nodes, and
   recently persisted reachable peers.
2. Perform iterative `FIND_NODE` toward own PeerId to populate k-buckets
3. After bootstrap, routing table maintains itself via periodic refresh

When a bootstrap multiaddr ends in `/p2p/<peer-id>`, the address without the
`/p2p` suffix is inserted into the Kademlia routing table before dialing. mDNS
discoveries and identify listen addresses are also inserted into Kademlia.

guix-p2p forces libp2p-kad server mode. The libp2p default starts in client
mode and auto-promotes only after address confidence improves; local private
VM tests do not always advertise public addresses, but they still need to
answer provider lookups.

### Bootstrap Peer Config

```toml
bootstrap_peers = "/ip4/p2p1.guix.example.org/udp/6881/quic-v1/p2p/12D3KooW...,/ip4/p2p2.guix.example.org/tcp/6881/p2p/12D3KooW..."
external_addresses = "/dns4/node.example.org/udp/6881/quic-v1"
```

There are no built-in default bootstrap peers yet. The code path is enabled by
`enable_default_bootstrap_peers = true` so project-operated bootnodes can be
added later without changing user config. Users can add known peers via the
`--bootstrap-peers` CLI flag or `bootstrap_peers` in the config file.

Reachable peers learned from successful dials, identify, and mDNS are persisted
under `cache_dir` when `peer_store_enabled = true`. Loopback, unspecified, and
local-interface addresses are not persisted or inserted into the active
Kademlia routing table. This avoids remembering stale peers from local test
daemons or VPN/TUN interfaces on the same host. Persisted peers are merged into
the next startup bootstrap set and do not rewrite the user's TOML config. If a
persisted address fails during an outbound dial, that address is removed from
the peer store and the active Kademlia routing table.

Nodes can also set `external_addresses` or `--external-addresses` when the
dialable address differs from the local listen address. Those addresses are
advertised through identify and included in provider records, which lets peers
found through the DHT dial the provider.

The dashboard uses `external_addresses` plus the local PeerId to show the exact
multiaddr a user can share with another peer.

## Kademlia Parameters

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| k (bucket size) | 20 | libp2p-kad default; good balance of redundancy vs overhead |
| α (concurrency) | 3 | libp2p-kad default; reasonable for QUIC transport |
| Key hash | SHA-256 | Native alignment with Guix nar-SHA-256 |
| Record TTL | 24 hours | Peer announcements expire; periodic republish keeps them alive |
| Republish interval | 22 hours | Republish before TTL expires (2h buffer) |
| Provider record limit | 20 | Max providers stored per key by kad nodes |

## Record Types

### Provider Records

Registered via `kad.start_providing(key)`. Value is the provider's PeerId.
libp2p-kad stores these in its MemoryStore (in-memory). For persistent storage
between restarts, a disk-backed Store implementation would be needed.

For the initial MVP, in-memory is sufficient:
- Providers re-announce on each startup
- Lookups work within the same session
- Background daemon mode (`--daemon`) keeps records alive long-term

### Potential Future: Signed Peer Metadata

Custom records stored via `kad.put_record()` with Ed25519 signatures:

```
Record {
  key: nar_hash
  value: {
    peer_id: PeerId
    addresses: ["/ip4/.../udp/.../quic-v1", ...]
    proof_blocks: [17, 42, 103]    // random block indices for anti-spam challenge
    available_until: timestamp
  }
  publisher: PeerId
  signature: Ed25519(peer_private_key, value)
}
```

This would enable:
- Multi-address announcements (peer behind changing IPs)
- Anti-spam challenge-response (request proof_blocks, verify)
- TTL expiry and re-announcement
- Cryptographic verification of the announcer

## Anti-Spam

Problem: a malicious peer could call `kad.start_providing()` for nars it
doesn't actually have, polluting lookup results and wasting download
attempts.

Current approach (MVP):
1. When downloading, attempt to connect to providers
2. If connection fails or peer doesn't respond to handshake/block requests,
   mark peer as failed
3. Failed or connection-backoff peers are skipped when building later
   handshake candidate lists

Enhanced approach (Phase 5 hardening):
1. After receiving providers list, challenge a random subset with proof-block requests
2. If peer responds with valid blocks (SHA-256 verified), trust the provider
3. If peer fails challenge or doesn't respond within timeout, blacklist for this nar
4. Maintain per-nar reputation scores across sessions

## NAT and Connectivity

libp2p-kad provider records store PeerIds, not addresses. Address resolution
happens via libp2p's identify protocol and dialing.

For MVP: peers behind NAT can connect outbound (to providers with open ports)
but cannot be dialed inbound. libp2p's relay protocol (post-MVP) solves this.

## Relationship to Existing Substitute Infrastructure

The DHT does NOT replace:
- Narinfo serving (still HTTP from official servers, tiny <500 bytes)
- Narinfo signature verification (still Guix SPKI/libgcrypt against ACL)
- Substitute URL configuration (still used for HTTP fallback)

The DHT ONLY replaces:
- Nar download transport (from HTTP GET to swarm block transfer)

## Monitoring

libp2p-kad exposes events through the swarm behaviour:

```rust
swarm.select_next_some().await match {
    SwarmEvent::Behaviour(GuixP2PEvent::Kad(KademliaEvent::RoutingUpdated { .. })) => {
        // Routing table changed
    }
    SwarmEvent::Behaviour(GuixP2PEvent::Kad(KademliaEvent::OutboundQueryProgressed { .. })) => {
        // DHT query in progress
    }
    _ => {}
}
```

Key metrics to track:
- Connected peer count
- Kademlia k-bucket occupancy
- Provider lookups per second
- Provider lookup latency (P50/P95)
- Provider lookup success rate
