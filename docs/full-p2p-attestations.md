# Full-P2P Attestations

This is a future implementation plan. It is not implemented.

The goal is a full P2P mode where users can fetch both substitute metadata and
NAR bytes without relying on centralized substitute servers. The trust model
must fail closed: peers can distribute data, but users only accept build
metadata when local policy approves the signing keys.

## Trust Model

- Use threshold reproducibility over trusted builder keys.
- Reuse Guix publish-style key material:
  - canonical S-expression public and secret keys
  - SPKI-style signatures
  - Guix ACL-compatible public key entries
- Keep attestation trust separate from `/etc/guix/acl`.
- Do not treat libp2p PeerIds, provider records, or peer reputation as build
  trust.
- Accept decentralized metadata only when an explicit local threshold is met by
  distinct authorized signing keys.

## Metadata Policy

Add a metadata source policy separate from `substitute_policy`.

Suggested values:

- `official-only`: current behavior.
- `official-first`: try official signed narinfo, then decentralized
  attestations.
- `attestation-first`: try decentralized attestations, then official signed
  narinfo.
- `attestation-only`: never fetch substitute-server narinfo for that item.

Full-P2P mode means:

```toml
substitute_policy = "p2p-only"
metadata_policy = "attestation-only"
attestation_threshold = 2
```

Decentralized metadata stays disabled until `attestation_threshold` and trusted
attestation keys are explicitly configured.

## Attestation Format

Use a narinfo-like signed text format.

Signed fields:

- `Attestation-Version`
- `StorePath`
- `NarHash`
- `NarSize`
- `References`
- `Deriver` when available
- `System` when available

Signature line:

```text
Signature: 1;<hostname>;<base64 canonical-sexp>
```

The signature covers the SHA-256 of the exact UTF-8 text above the
`Signature:` line, matching Guix narinfo behavior.

## User Interfaces

Suggested config:

```toml
metadata_policy = "official-first"
attestation_acl_path = "~/.config/guix-p2p/attestation-acl"
attestation_threshold = 2
attestation_cache_ttl_secs = 3600
```

Suggested CLI:

```sh
guix-p2p attest create STORE_PATH --private-key /etc/guix/signing-key.sec --output FILE
guix-p2p attest verify FILE --acl PATH
guix-p2p attest import FILE
guix-p2p attest publish FILE --socket PATH
guix-p2p attest explain STORE_PATH
```

For v1 authoring, call Guix's existing signing machinery rather than parsing
and signing Guix secret keys in Rust.

## Network Discovery

Use DHT provider discovery plus explicit attestation fetch.

- Provider key: `sha256("guix-p2p-attestation-v1:" + store_hash_part)`.
- Request-response protocol: `"/guix/attestations/0.1.0"`.
- Peers announce that they have attestations for a store hash.
- Fetchers request signed attestation lists from provider peers.
- Fetchers verify all signatures and threshold policy locally.

Avoid storing multi-writer attestation sets directly as Kademlia value records
in v1. Provider records naturally handle many peers per key; attestation lists
can then be fetched and verified from those peers.

## Acceptance Rules

- Reject unsigned, malformed, expired, or untrusted attestations.
- Count quorum by distinct authorized public keys.
- Multiple signatures from the same key count once.
- Conflicting trusted attestations for the same store path fail closed.
- Final NAR bytes must still match the attested `NarHash`.
- Peer reputation remains transport-only.

## E2E Proof

The first proof should demonstrate an attestation-only `hello` substitute:

- Alice and Charles each sign the same `hello` store path with distinct Guix
  publish-style keys.
- Bob trusts both public keys in `attestation-acl`.
- Bob sets `attestation_threshold = 2`, `metadata_policy = "attestation-only"`,
  and `substitute_policy = "p2p-only"`.
- Bob fetches `hello` through P2P without substitute-server narinfo for that
  item.

## Deferred

- Web-of-trust delegation.
- Rust-side signing support for Guix secret keys.
- Upstream Guix integration hooks.
- Full provenance, build logs, channel signatures, and build environment
  attestations in the acceptance path.
