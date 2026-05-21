# Full-P2P Attestations

This is a future implementation plan. It is not implemented.

The goal is a full P2P mode where users can fetch both substitute metadata and
NAR bytes without relying on centralized substitute servers.

Peers are untrusted byte transport. A malicious peer may lie about availability,
stall, or send corrupt blocks; the downloader still accepts a substitute only
when local policy accepts signed reproducible-build attestations and the
downloaded NAR bytes match the accepted hash.

This is not fully trustless binary correctness. A downloader cannot know the
`NarHash` for an arbitrary build from source alone without building it locally.
Instead, guix-p2p replaces single metadata-server trust with a Bitcoin
Core-style reproducible-build quorum: independent builders build the same Guix
derivation, sign the hashes they produced, and users choose which builder keys
and threshold they trust.

## Goal

Implement a metadata path that can replace substitute-server narinfo lookups for
users who explicitly opt in.

The completed path should let a user:

- Build or otherwise possess a Guix store item.
- Create a signed attestation for the exact derivation output and NAR hash.
- Seed the NAR bytes and signed attestation to the P2P network.
- Download from peers with `metadata_policy = "attestation-only"` and
  `substitute_policy = "p2p-only"`.
- Accept the substitute only when enough trusted builder keys agree and the
  downloaded bytes hash to the accepted `NarHash`.

The first implementation should prove this with one package, then one profile
closure, then a full system closure.

## Trust Model

- Use threshold reproducibility over trusted builder keys. Without local builds,
  the trust root is the user's configured attestation keys and threshold.
- Reuse Guix publish-style key material:
  - canonical S-expression public and secret keys
  - SPKI-style signatures
  - Guix ACL-compatible public key entries
- Keep attestation trust separate from `/etc/guix/acl`.
- Do not treat libp2p PeerIds, provider records, or peer reputation as build
  trust.
- Accept decentralized metadata only when an explicit local threshold is met by
  distinct authorized signing keys that agree on the same derivation output.
- Fail closed when trusted attestations disagree about the derivation, store
  path, NAR hash, NAR size, or references.

The security split is:

- P2P transport is trustless: peers never establish content trust.
- Binary correctness is trust-minimized: users no longer need one centralized
  narinfo server, but they still choose attestation keys and threshold policy.
- Guix source and channel trust remain outside guix-p2p. Attestations bind a
  derivation output to produced bytes; they do not decide which source code a
  user should trust.

## Non-Goals

- Do not prove that a builder used an untampered machine.
- Do not prove that the source is trustworthy.
- Do not require server-side builders, transparency logs, TEEs, TPMs, Sigstore,
  SLSA, in-toto, SCITT, or build-log verification in v1.
- Do not publish mutable attestation sets as Kademlia records.
- Do not make libp2p identity part of build trust.

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

Implementation detail:

- `substitute_policy` still decides where NAR bytes come from.
- `metadata_policy` decides where `NarHash`, `NarSize`, `Deriver`, and
  references come from.
- `official-only` should remain the default.
- `attestation-only` must fail closed if the configured threshold cannot be
  met.
- `official-first` and `attestation-first` are migration modes, not full P2P.

## Attestation Format

Use a narinfo-like signed text format. The attestation claim is: "I built this
Guix derivation output and got this NAR hash."

Signed fields:

- `Attestation-Version`
- `StorePath`
- `NarHash`
- `NarSize`
- `References`
- `Deriver`
- `System` when available
- `Output` when available
- `CreatedAt` as Unix seconds

Signature line:

```text
Signature: 1;<hostname>;<base64 canonical-sexp>
```

The signature covers the SHA-256 of the exact UTF-8 text above the
`Signature:` line, matching Guix narinfo behavior.

`Deriver` should be mandatory when available. The accepted claim is about a
Guix derivation output, not arbitrary bytes attached to a store path.

Canonical v1 field order:

```text
Attestation-Version: 1
StorePath: /gnu/store/...
Deriver: /gnu/store/...drv
Output: out
System: x86_64-linux
NarHash: sha256:...
NarSize: 123
References: /gnu/store/... /gnu/store/...
CreatedAt: 1780000000
Signature: 1;builder-name;<base64 canonical-sexp>
```

`URL`, `Compression`, and `FileSize` are intentionally excluded. Attestations
authenticate the uncompressed NAR byte stream, not a transport location.

`CreatedAt` supports cache eviction and diagnostics. v1 acceptance should not
reject old attestations solely because they are old unless the user configures a
maximum age.

## Attestation Identity

Use the same public-key encoding that `narinfo.rs` already accepts from Guix ACL
files:

```scheme
(public-key
 (ecc
  (curve Ed25519)
  (q #...#)))
```

The attestation ACL is a separate file:

```text
~/.config/guix-p2p/attestation-acl
```

The key ID displayed in logs and `attest explain` should be the lowercase hex
SHA-256 of the raw Ed25519 public key bytes, truncated to 16 hex characters for
human output.

Threshold counting rules:

- Count distinct authorized public keys, not hostnames.
- Count one valid attestation per key.
- If one key signs multiple conflicting claims for the same store path, mark
  that key as conflicting for the decision and fail the decision.
- A libp2p peer can relay attestations from many keys; the peer itself is not
  counted.

## Data Model

Add an attestation module instead of overloading `Narinfo`.

Suggested Rust types:

```rust
pub struct Attestation {
    pub store_path: String,
    pub deriver: Option<String>,
    pub output: Option<String>,
    pub system: Option<String>,
    pub nar_hash: String,
    pub nar_size: u64,
    pub references: Vec<String>,
    pub created_at: u64,
    pub signed_portion: String,
    pub signature: String,
}

pub struct AcceptedMetadata {
    pub store_path: String,
    pub deriver: Option<String>,
    pub nar_hash: String,
    pub nar_size: u64,
    pub references: Vec<String>,
    pub trusted_signers: Vec<KeyId>,
}

pub enum MetadataSource {
    OfficialNarinfo,
    AttestationQuorum,
    LocalNarinfo,
}
```

`AcceptedMetadata` should be convertible to the existing substitute response
shape so the daemon can keep using the same `success sha256:<hash> <size>` and
info replies.

## Local Storage

Store imported or locally created attestations under the cache directory:

```text
$XDG_CACHE_HOME/guix-p2p/attestations/v1/<store-hash-part>/<key-id>.att
```

Rules:

- The filename is not trusted; always parse and verify file content.
- Multiple attestations from the same key for the same store hash are allowed on
  disk only if they are byte-identical. Conflicting imports should be rejected
  and reported.
- Cache entries can be rebuilt from peer fetches, so this is not durable user
  configuration.
- Trusted key material remains in `attestation_acl_path`, not in the cache.

## Metadata Resolver

Introduce a resolver layer between daemon query handling and byte download:

```text
daemon request
  -> MetadataResolver::resolve(store_path)
  -> AcceptedMetadata
  -> P2P or HTTP NAR download
  -> final NarHash check
```

Resolver behavior by policy:

| Policy | Resolution order |
|--------|------------------|
| `official-only` | local narinfo cache, official narinfo |
| `official-first` | local narinfo cache, official narinfo, attestation quorum |
| `attestation-first` | local attestation cache, peer attestations, official narinfo |
| `attestation-only` | local attestation cache, peer attestations |

Local benchmark metadata should keep working through the existing
`local_narinfo_path`. It is explicit harness input, not decentralized metadata.

Failure modes:

- No trusted quorum: reply `not-found`.
- Trusted conflict: reply `not-found` and log a warning with the conflicting key
  IDs.
- Accepted metadata but bad NAR bytes: reply `hash-mismatch`.
- Malformed untrusted peer data: ignore and continue until timeout.

## User Interfaces

Suggested config:

```toml
metadata_policy = "official-first"
attestation_acl_path = "~/.config/guix-p2p/attestation-acl"
attestation_threshold = 2
attestation_cache_ttl_secs = 3600
attestation_max_age_secs = 0
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

`attest create` should build or inspect the local store item, compute the NAR
hash and size, include the deriver and references, and sign the result with the
builder's configured key. The command should not imply that the signer is a
trusted build authority; trust is assigned only by the downloader's
`attestation_acl_path` and threshold policy.

Command behavior:

- `attest create` writes an attestation file and does not publish by default.
- `attest import` verifies syntax and stores the file in the local attestation
  cache even if the key is not trusted locally.
- `attest verify` checks signature validity and threshold policy when enough
  files are passed.
- `attest publish` announces the attestation provider key and serves matching
  attestations through the daemon.
- `attest explain` shows accepted, ignored, and conflicting attestations for a
  store path.

## Signing Strategy

For v1, avoid implementing Guix secret-key parsing in Rust.

Preferred path:

- Generate the unsigned attestation text in Rust.
- Ask Guix/libgcrypt tooling to produce the same SPKI-style signature format as
  narinfo signatures.
- Parse and verify the resulting signed text with the Rust verifier.

This keeps signing compatible with Guix key material while limiting Rust crypto
work to the verification path we already exercise for narinfos.

Open implementation detail: identify the smallest stable Guix command or Guile
entry point that signs arbitrary text with a Guix signing key. If there is no
clean public command, add a small Guile helper shipped with guix-p2p.

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

Request-response protocol:

```rust
enum AttestationRequest {
    List {
        store_hash_part: String,
        limit: u32,
    },
}

enum AttestationResponse {
    List {
        attestations: Vec<String>,
    },
    Error {
        message: String,
    },
}
```

The response carries signed text attestations. The fetcher must cap response
size and attestation count before parsing:

- Maximum attestations per peer: 64.
- Maximum single attestation size: 16 KiB.
- Maximum response size: 1 MiB.
- Per-peer request timeout: reuse the existing request timeout unless a tighter
  attestation timeout is added.

Provider key derivation should use the store path hash part, not `NarHash`.
During metadata resolution the downloader knows the store path but does not yet
know the accepted `NarHash`.

## Acceptance Rules

- Reject unsigned, malformed, expired, or untrusted attestations.
- Count quorum by distinct authorized public keys.
- Multiple signatures from the same key count once.
- Accept quorum only when trusted attestations agree on `Deriver`, `StorePath`,
  `NarHash`, `NarSize`, and `References`.
- Conflicting trusted attestations for the same derivation output fail closed.
- Final NAR bytes must still match the attested `NarHash`.
- Peer reputation remains transport-only.

Decision algorithm:

1. Parse all local and fetched attestations for the requested store path.
2. Verify each signature against `attestation_acl_path`.
3. Drop attestations from keys outside the ACL.
4. Group trusted attestations by claim tuple:
   `(Deriver, StorePath, NarHash, NarSize, References)`.
5. Fail closed if any trusted key signs more than one claim tuple.
6. Accept the only tuple that reaches `attestation_threshold`.
7. Fail closed if more than one tuple reaches the threshold.
8. Convert the accepted tuple into `AcceptedMetadata`.

The final NAR hash verification remains the enforcement point for malicious
transport peers.

## Implementation Phases

### Phase 1: Parser and Quorum Engine

Files likely touched:

- `src/attestation.rs`
- `src/narinfo.rs`
- `src/config.rs`
- `src/main.rs`
- `docs/architecture.md`

Tasks:

- Add `Attestation` parser for the signed text format.
- Reuse Guix ACL parsing and SPKI verification logic from `narinfo.rs`.
- Return the verifying key or key ID from signature verification so quorum
  counting can distinguish signers.
- Add `MetadataPolicy` config with default `official-only`.
- Add `attestation_acl_path`, `attestation_threshold`,
  `attestation_cache_ttl_secs`, and `attestation_max_age_secs`.
- Unit-test parsing, signature rejection, threshold success, threshold miss,
  duplicate signer handling, and trusted conflict handling.

### Phase 2: Local Authoring and Import

Files likely touched:

- `src/main.rs`
- `src/attestation.rs`
- `src/nar_store.rs`
- `docs/architecture.md`

Tasks:

- Add `guix-p2p attest create`.
- Compute NAR hash and size from the local store item with the same byte stream
  used for seeding.
- Include deriver and references from Guix store metadata.
- Sign through a Guile helper or stable Guix signing entry point.
- Add `attest import`, `attest verify`, and `attest explain`.
- Store imported attestations in the cache layout above.

### Phase 3: Local Attestation-Only Substitution

Files likely touched:

- `src/daemon.rs`
- `src/http_client.rs`
- `src/attestation.rs`
- `src/config.rs`
- `e2e/src/main.rs`

Tasks:

- Add `MetadataResolver`.
- Make daemon `have`, `info`, and `substitute` use `AcceptedMetadata`.
- Support `metadata_policy = "attestation-only"` with local cached
  attestations.
- Keep `official-only` behavior unchanged.
- Add an e2e test where Bob imports Alice and Charles attestations and fetches
  bytes from a local P2P seeder without HTTP narinfo for the item.

### Phase 4: P2P Attestation Discovery

Files likely touched:

- `src/behaviour.rs`
- `src/runtime.rs`
- `src/channel.rs`
- `src/dht.rs`
- `src/attestation.rs`
- `docs/swarm-protocol.md`

Tasks:

- Add `"/guix/attestations/0.1.0"` request-response behavior.
- Add provider announcements for attestation keys.
- Fetch attestation lists from provider peers during metadata resolution.
- Cache valid fetched attestations locally.
- Apply response size limits before parsing.
- Add integration tests for peer fetch, malformed peer response, insufficient
  quorum, conflict, and successful quorum.

### Phase 5: Full Closure Proof

Files likely touched:

- `e2e/src/main.rs`
- `docs/benchmarks.md`
- `docs/full-p2p-attestations.md`

Tasks:

- Extend e2e fixtures from one package to a profile closure.
- Extend again to a system closure once runtime is stable.
- Report how many paths resolved through attestation quorum.
- Report how many NAR bytes came from P2P under `attestation-only`.
- Verify no substitute-server narinfo requests are made for attested items.

## E2E Proof

The first proof should demonstrate an attestation-only `hello` substitute:

- Alice and Charles independently build the same `hello` derivation and each
  sign the resulting store path, references, NAR size, and NAR hash with
  distinct Guix publish-style keys.
- Bob trusts both public keys in `attestation-acl`.
- Bob sets `attestation_threshold = 2`, `metadata_policy = "attestation-only"`,
  and `substitute_policy = "p2p-only"`.
- Bob fetches `hello` through P2P without substitute-server narinfo for that
  item.
- Bob accepts the metadata only if Alice and Charles agree on the same
  derivation output and accepts the bytes only if the downloaded NAR hashes to
  the attested `NarHash`.

The second proof should cover a closure:

- Alice and Charles sign every path in a small profile closure.
- Bob starts with an empty store for those paths.
- Bob resolves all metadata through attestations.
- Bob downloads all NAR bytes through P2P.
- The test fails if any attested path requires official narinfo.

The third proof should cover a full system closure. It should start as a manual
or ignored CI benchmark until runtime and storage costs are understood.

## Deferred

- Web-of-trust delegation.
- Rust-side signing support for Guix secret keys.
- Upstream Guix integration hooks.
- SLSA, in-toto, Sigstore, SCITT, TEE-backed builds, TPM evidence, build logs,
  channel signatures, and build environment attestations in the acceptance path.
- Public append-only transparency logs.
- Revocation beyond editing the local attestation ACL.
