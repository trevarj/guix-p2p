# Phase 4: HTTP Fallback + Narinfo Verification

## Prerequisites
Phase 3 is complete: swarm downloader works, block exchange works between peers.

## Goal
Implement reliable HTTP fallback when the swarm is unavailable or fails.
Implement proper narinfo parsing and Ed25519 signature verification.
Ensure the substituter never leaves the daemon hanging — it always returns success, not-found, or hash-mismatch.

## Tasks

### 1. src/narinfo.rs — Full implementation

Complete the narinfo parser and signature verifier.

**Narinfo format:**
```
StorePath: /gnu/store/<hash>-<name>-<version>
URL: nar/<compression>/<hash>-<name>-<version>
Compression: <compression>
FileSize: <bytes>
NarHash: sha256:<base32-hash>
NarSize: <bytes>
References: <ref1> <ref2> ...
Deriver: <deriver-path>
System: <system>
Signature: 1;<hostname>;<base64-canonical-sexp>
```

Key detail from Guix source: fields above `Signature:` are the signed portion.
Fields below it (URL, Compression, FileSize) are NOT signed and can be modified.

**Data structures:**
```rust
#[derive(Debug, Clone)]
pub struct Narinfo {
    pub store_path: String,
    pub nar_hash: String,        // "sha256:<base32>"
    pub nar_hash_algo: String,   // "sha256"
    pub nar_hash_value: String,  // "<base32>"
    pub nar_size: u64,
    pub references: Vec<String>,
    pub deriver: Option<String>,
    pub system: Option<String>,
    pub urls: Vec<NarUrl>,
    /// The raw text of the signed portion (for signature verification).
    pub signed_portion: String,
    /// Raw signature string: "1;<hostname>;<base64>"
    pub signature: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NarUrl {
    pub url: String,
    pub compression: String,
    pub file_size: u64,
}
```

**Parser:**
```rust
pub fn parse_narinfo(input: &str) -> Result<Narinfo, ParseError> {
    // Split on "Signature:" line boundary
    // The signed portion is everything BEFORE "Signature:"
    // URL/Compression/FileSize are AFTER (not signed)
    // Parse key: value pairs in each section
    // References are space-separated on one line
}
```

**Signature verification:**
The signature is a SPKI-style canonical sexp:
```
(signature
  (data (flags pkcs1) (hash sha256 #<32-byte-hash>#))
  (sig-val (eddsa (r #...) (s #...)))
  (public-key (ecc (curve Ed25519) (q #...))))
```

Simplified for our purposes: the narinfo signature verification works as follows:
1. Compute SHA-256 of the signed portion (the text above "Signature:" line)
2. The signature is base64-encoded canonical sexp containing:
   - The signed hash (must match our computed hash)
   - The Ed25519 signature value (must verify against the public key)
   - The public key (must be in our authorized keys list)
3. Verify: hash_match && key_authorized && signature_valid

```rust
pub fn verify_narinfo_signature(
    narinfo: &Narinfo,
    authorized_keys: &[ed25519_dalek::VerifyingKey],
) -> Result<(), SignatureError> {
    // 1. Compute SHA-256 of signed_portion text
    let computed_hash = sha2::Sha256::digest(narinfo.signed_portion.as_bytes());
    
    // 2. Decode the base64 signature
    // Format: "1;<hostname>;<base64-canonical-sexp>"
    let sig_parts: Vec<&str> = narinfo.signature?.splitn(3, ';').collect();
    let sig_bytes = base64::decode(sig_parts[2])?;
    
    // 3. Parse the canonical sexp to extract (hash, sig, public-key)
    // Use a minimal SPKI parser or the ed25519-dalek crate directly
    // The canonical sexp has a known structure we can parse
    
    // 4. Verify hash matches
    // 5. Verify public key is in authorized_keys
    // 6. Verify Ed25519 signature
    
    Ok(())
}
```

**Caching:**
```rust
pub struct NarinfoCache {
    cache_dir: PathBuf,
}

impl NarinfoCache {
    pub fn get(&self, hash_part: &str) -> Option<Narinfo>;
    pub fn put(&self, hash_part: &str, narinfo: &Narinfo) -> io::Result<()>;
    pub fn get_negative(&self, hash_part: &str) -> Option<bool>;
    pub fn put_negative(&self, hash_part: &str) -> io::Result<()>;
}
```
Cache positive results for 36 hours, negative for 2 minutes.
Store as JSON files in `{cache_dir}/narinfo/` keyed by the 32-char hash part.

### 2. src/fallback.rs — Full HTTP implementation

```rust
/// Default substitute URLs (matches Guix defaults).
const DEFAULT_SUBSTITUTE_URLS: &[&str] = &[
    "https://bordeaux.guix.gnu.org",
    "https://ci.guix.gnu.org",
];

/// Fetch narinfo for a store item hash from substitute servers.
pub async fn fetch_narinfo(
    config: &Config,
    hash_part: &str,
) -> Result<Narinfo, FetchError> {
    let urls = config.substitute_urls();
    for base_url in &urls {
        let url = format!("{}/{}.narinfo", base_url, hash_part);
        match fetch_narinfo_from_url(&url).await {
            Ok(narinfo) => {
                // Verify signature before returning
                verify_narinfo_signature(&narinfo, &config.authorized_keys()?)?;
                return Ok(narinfo);
            }
            Err(e) => {
                tracing::warn!("Failed to fetch narinfo from {}: {}", url, e);
                continue;
            }
        }
    }
    Err(FetchError::NotFound)
}
```

```rust
/// Download a nar from substitute servers via HTTP.
/// Returns (hash, nar_size) on success.
pub async fn download_nar(
    config: &Config,
    narinfo: &Narinfo,
    dest_path: &Path,
) -> Result<(String, u64), DownloadError> {
    // Build URL from narinfo's preferred URL
    let base = narinfo.store_path_parent_url();
    let url = format!("{}/{}", base, narinfo.best_url().url);
    
    let response = reqwest::get(&url).await?;
    
    // Decompress and write to dest_path
    // Compute hash while writing
    // Verify hash matches narinfo.nar_hash after download
}
```

**Decompression support:**
Support `gzip`, `lzip`, `zstd`, `none`.
Use `async-compression` crate or decompress at the I/O level:
```rust
async fn decompress_and_verify(
    response: reqwest::Response,
    compression: &str,
    expected_hash: &str,
    dest_path: &Path,
) -> Result<(String, u64), DownloadError> {
    let reader = response.bytes_stream();
    
    // Wrap in decompression layer
    let decompressed = match compression {
        "gzip" => Box::new(AsyncGzipDecoder::new(reader)),
        "zstd" => Box::new(AsyncZstdDecoder::new(reader)),
        "lzip" => return Err(DownloadError::Unsupported("lzip in async".into())), // defer or use sync
        _ => Box::new(reader), // "none" or unknown → raw
    };
    
    // Stream to file while hashing
    let mut file = tokio::fs::File::create(dest_path).await?;
    let mut hasher = sha2::Sha256::new();
    
    // Read chunks, write to file, update hasher
    // ...
    
    // Verify hash
    verify_hash(&hasher, expected_hash)?;
    Ok((format!("sha256:{:x}", hasher.finalize()), file_size))
}
```

### 3. src/fallback.rs — Narinfo fetch from URL

```rust
async fn fetch_narinfo_from_url(url: &str) -> Result<Narinfo, FetchError> {
    let response = reqwest::get(url).await?;
    let body = response.text().await?;
    
    // Check for X-Baking header (server is preparing the resource)
    // If present, we might want to retry or skip
    
    parse_narinfo(&body).map_err(FetchError::Parse)
}
```

### 4. src/daemon.rs — Complete substitute handling

Update `handle_substitute()` with the full flow:

```rust
async fn handle_substitute(
    swarm: &mut Swarm<GuixP2PBehaviour>,
    config: &Config,
    path: &str,
    dest: &str,
) -> anyhow::Result<()> {
    let hash_part = extract_hash_part(path)?;
    let dest_path = PathBuf::from(dest);
    
    // 1. Fetch narinfo (needed for nar size, hash, and URLs in all cases)
    let narinfo = match fetch_or_cache_narinfo(config, &hash_part).await {
        Ok(info) => info,
        Err(e) => {
            tracing::warn!("Cannot fetch narinfo for {}: {}", hash_part, e);
            write_reply_fd4(&format!("not-found {}", path))?;
            return Ok(());
        }
    };
    
    // 2. Try swarm download
    let providers = dht::get_providers(swarm, &hash_part);
    if providers.len() >= 3 {
        tracing::info!("Found {} providers for {}, attempting swarm download", providers.len(), hash_part);
        match downloader::download_nar(swarm, config, &narinfo.nar_hash_value, narinfo.nar_size, &dest_path, providers).await {
            Ok(_) => {
                write_reply_fd4(&format!("success {} {}", narinfo.nar_hash, narinfo.nar_size))?;
                return Ok(());
            }
            Err(SwarmError::Stalled) => {
                tracing::warn!("Swarm download stalled, falling back to HTTP");
            }
            Err(e) => {
                tracing::warn!("Swarm download failed: {}, falling back to HTTP", e);
            }
        }
    } else {
        tracing::info!("Only {} providers for {}, using HTTP directly", providers.len(), hash_part);
    }
    
    // 3. HTTP fallback
    match fallback::download_nar(config, &narinfo, &dest_path).await {
        Ok((hash, size)) => {
            write_reply_fd4(&format!("success {} {}", hash, size))?;
        }
        Err(e) => {
            tracing::error!("HTTP fallback failed for {}: {}", hash_part, e);
            write_reply_fd4(&format!("not-found {}", path))?;
        }
    }
    
    Ok(())
}
```

### 5. src/config.rs — Update

Add:
```rust
pub struct Config {
    // ... existing fields ...
    pub substitute_urls: Vec<String>,
    pub acl_path: PathBuf, // /etc/guix/acl or ~/.config/guix/acl
}

impl Config {
    pub fn substitute_urls(&self) -> Vec<String> { ... }
    pub fn authorized_keys(&self) -> Result<Vec<ed25519_dalek::VerifyingKey>> { 
        // Parse /etc/guix/acl to extract authorized public keys
        // The ACL is a Scheme s-expression. We can parse a simplified subset.
        // Format: (acl (entry (public-key (ecc (curve Ed25519) (q #<32bytes>#))) (tag (guix import))))
    }
}
```

### 6. src/main.rs — Trace output

Daemon expects substitute progress traces on stdout (fd 1) in this format:
```
@ download-started /gnu/store/...-foo https://ci.guix.gnu.org/nar/lzip/...-foo 12345
@ download-progress /gnu/store/...-foo https://ci.guix.gnu.org/... 12345 6789
@ download-succeeded /gnu/store/...-foo https://ci.guix.gnu.org/... 12345
```

Wire these into both swarm downloader and HTTP fallback:
- Swarm: emit traces as blocks download, with a pseudo-URL like `p2p://<peer_id>/<nar_hash>`
- HTTP fallback: emit standard traces with the actual URL

## Deliverables
- Full narinfo parser with correct signed/unsigned field handling
- Ed25519 signature verification with key authorization
- Narinfo caching (positive + negative)
- HTTP nar download with decompression and hash verification
- Automatic swarm → HTTP fallback with clear trace output
- Binary always returns a timely reply to the daemon

## Verification
- `cargo fmt` passes
- `cargo clippy --all-targets --all-features -- -D warnings` passes
- `cargo test` passes
- Narinfo parsing test with real narinfo from ci.guix.gnu.org
- Signature verification test with known good/bad signatures
- HTTP fallback test: download a real nar from bordeaux.guix.gnu.org
- Integration test: swarm fails → HTTP succeeds → daemon gets success reply
