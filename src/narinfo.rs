use std::collections::HashMap;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

/// A simple TTL-based cache for narinfos keyed by hash part.
#[derive(Debug, Default)]
pub struct NarinfoCache {
    entries: HashMap<String, (tokio::time::Instant, Narinfo)>,
    ttl: tokio::time::Duration,
}

impl NarinfoCache {
    pub fn new(ttl_secs: u64) -> Self {
        NarinfoCache { entries: HashMap::new(), ttl: tokio::time::Duration::from_secs(ttl_secs) }
    }

    pub fn get(&self, hash_part: &str) -> Option<Narinfo> {
        let (_expiry, info) = self.entries.get(hash_part)?;
        Some(info.clone())
    }

    pub fn put(&mut self, hash_part: String, narinfo: Narinfo) {
        self.entries.insert(hash_part, (tokio::time::Instant::now() + self.ttl, narinfo));
    }
}

#[derive(Debug, Clone)]
pub struct Narinfo {
    pub store_path: String,
    pub nar_hash: String,
    pub nar_size: u64,
    pub references: Vec<String>,
    pub deriver: Option<String>,
    pub urls: Vec<NarUrl>,
    pub signed_portion: String,
    pub signature: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NarUrl {
    #[allow(dead_code)]
    pub url: String,
    #[allow(dead_code)]
    pub compression: String,
    pub file_size: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("parse error: {0}")]
    #[allow(dead_code)]
    Parse(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn parse_narinfo(input: &str) -> Result<Narinfo, ParseError> {
    let (signed_portion, remainder) = if let Some(sig_pos) = input.find("Signature:") {
        let (above, below) = input.split_at(sig_pos);
        (above.trim_end().to_string(), Some(below.to_string()))
    } else {
        (input.trim().to_string(), None)
    };

    let mut fields: HashMap<String, String> = HashMap::new();
    let mut urls: Vec<(String, String, u64)> = Vec::new();
    let mut url_list: Vec<String> = Vec::new();
    let mut compressions: Vec<String> = Vec::new();
    let mut file_sizes: Vec<u64> = Vec::new();

    for line in signed_portion.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            fields.insert(key, value);
        }
    }

    if let Some(remainder) = &remainder {
        for line in remainder.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("Signature:") {
                continue;
            }
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "URL" => url_list.push(value.to_string()),
                    "Compression" => compressions.push(value.to_string()),
                    "FileSize" => {
                        file_sizes.push(value.parse::<u64>().unwrap_or(0));
                    },
                    _ => {},
                }
            }
        }
    }

    for (i, _url_item) in url_list.iter().enumerate() {
        urls.push((
            url_list[i].clone(),
            compressions.get(i).cloned().unwrap_or_else(|| "none".into()),
            *file_sizes.get(i).unwrap_or(&0),
        ));
    }

    let signature = remainder.as_ref().and_then(|r| {
        r.lines()
            .find(|l| l.trim().starts_with("Signature:"))
            .map(|l| l.trim().strip_prefix("Signature:").unwrap_or("").trim().to_string())
    });

    let references: Vec<String> = fields
        .get("References")
        .map(|r| r.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();

    Ok(Narinfo {
        store_path: fields.get("StorePath").cloned().unwrap_or_default(),
        nar_hash: fields.get("NarHash").cloned().unwrap_or_default(),
        nar_size: fields.get("NarSize").and_then(|s| s.parse().ok()).unwrap_or(0),
        references,
        deriver: fields.get("Deriver").cloned(),
        urls: urls
            .into_iter()
            .map(|(url, compression, file_size)| NarUrl { url, compression, file_size })
            .collect(),
        signed_portion,
        signature,
    })
}

/// Load authorized public keys from the Guix ACL file.
/// Format: `(acl (entry (public-key (ecc (curve Ed25519) (q #<32-byte-hex>#))) …))`
pub fn load_acl_keys(path: &std::path::Path) -> Result<Vec<VerifyingKey>, ParseError> {
    let raw = std::fs::read_to_string(path)?;
    let mut keys = Vec::new();

    let mut pos = 0;
    while let Some(q_start) = raw[pos..].find("(q #") {
        let abs_start = pos + q_start + 4; // after "(q #"
        let rest = &raw[abs_start..];
        let hex_end = rest.find('#').ok_or(ParseError::Parse("unterminated q value".into()))?;
        let hex_str = &rest[..hex_end];

        let bytes = hex::decode(hex_str)
            .map_err(|e| ParseError::Parse(format!("invalid hex in ACL: {}", e)))?;
        if bytes.len() != 32 {
            return Err(ParseError::Parse(format!(
                "expected 32-byte key, got {} bytes",
                bytes.len()
            )));
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes);
        let key = VerifyingKey::from_bytes(&key_bytes)
            .map_err(|e| ParseError::Parse(format!("invalid Ed25519 key in ACL: {}", e)))?;
        keys.push(key);

        pos = abs_start + hex_end + 1;
    }

    Ok(keys)
}

/// Verify the narinfo's Ed25519 signature against a list of authorized keys.
/// Returns true if any authorized key validates the signature.
pub fn verify_narinfo_signature(narinfo: &Narinfo, keys: &[VerifyingKey]) -> bool {
    let sig_str = match &narinfo.signature {
        Some(s) => s,
        None => return false,
    };

    // Parse "1;hostname;base64-signature"
    let parts: Vec<&str> = sig_str.splitn(3, ';').collect();
    if parts.len() < 3 {
        return false;
    }

    let b64 = parts[2];
    let sexp_bytes = match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let sexp = String::from_utf8_lossy(&sexp_bytes);

    // Extract (r #hex#) from canonical sexp
    let r_hex = match extract_sexp_field(&sexp, "r") {
        Some(h) => h,
        None => return false,
    };
    let s_hex = match extract_sexp_field(&sexp, "s") {
        Some(h) => h,
        None => return false,
    };

    let r_bytes = match hex::decode(&r_hex) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let s_bytes = match hex::decode(&s_hex) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut sig_bytes = [0u8; 64];
    sig_bytes[..r_bytes.len().min(32)].copy_from_slice(&r_bytes[..r_bytes.len().min(32)]);
    sig_bytes[32..32 + s_bytes.len().min(32)].copy_from_slice(&s_bytes[..s_bytes.len().min(32)]);

    let signature = match Signature::from_slice(&sig_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let hash = Sha256::digest(narinfo.signed_portion.as_bytes());

    keys.iter().any(|key| key.verify(&hash, &signature).is_ok())
}

/// Extract a field value `(field #hex#)` from a canonical s-expression string.
fn extract_sexp_field(sexp: &str, field: &str) -> Option<String> {
    let needle = format!("({} #", field);
    let start = sexp.find(&needle)? + needle.len();
    let rest = &sexp[start..];
    let end = rest.find('#')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_with_signature_and_urls() {
        let data = "\
StorePath: /gnu/store/abcdef-vim-9.0
NarHash: sha256:abcdef123
NarSize: 20000000
References: /gnu/store/xyz-lib
Deriver: /gnu/store/xyz-vim.drv
System: x86_64-linux
Signature: 1;mockhost;dGVzdA==
URL: nar/gzip/abcdef-vim-9.0
Compression: gzip
FileSize: 5000000
";
        let info = parse_narinfo(data).unwrap();
        assert_eq!(info.nar_hash, "sha256:abcdef123");
        assert_eq!(info.nar_size, 20000000);
        assert_eq!(info.references.len(), 1);
        assert_eq!(info.urls.len(), 1);
        assert_eq!(info.urls[0].compression, "gzip");
        assert_eq!(info.urls[0].file_size, 5000000);
        assert!(info.signature.is_some());
    }

    #[test]
    fn test_parse_multiple_urls() {
        let data = "\
StorePath: /gnu/store/abc-test
NarHash: sha256:abc
NarSize: 100
Signature: 1;host;sig
URL: nar/gzip/abc-test
Compression: gzip
FileSize: 1000
URL: nar/zstd/abc-test
Compression: zstd
FileSize: 800
URL: nar/lzip/abc-test
Compression: lzip
FileSize: 500
";
        let info = parse_narinfo(data).unwrap();
        assert_eq!(info.urls.len(), 3);
        assert_eq!(info.urls[0].compression, "gzip");
        assert_eq!(info.urls[1].compression, "zstd");
        assert_eq!(info.urls[2].compression, "lzip");
    }

    #[test]
    fn test_parse_minimal_narinfo() {
        let data = "\
StorePath: /gnu/store/abc-test
NarHash: sha256:abc
NarSize: 100
";
        let info = parse_narinfo(data).unwrap();
        assert_eq!(info.store_path, "/gnu/store/abc-test");
        assert!(info.references.is_empty());
        assert!(info.deriver.is_none());
        assert!(info.urls.is_empty());
        assert!(info.signature.is_none());
    }

    #[test]
    fn test_parse_no_signature_boundary() {
        // Fields after Signature line in real narinfos are NOT signed.
        // URL/Compression/FileSize after signature should be parsed.
        let data = "\
StorePath: /gnu/store/abc-test
NarHash: sha256:abc
NarSize: 100
Signature: 1;host;deadbeef
URL: nar/gzip/abc-test
Compression: gzip
FileSize: 42
";
        let info = parse_narinfo(data).unwrap();
        assert_eq!(info.urls.len(), 1);
        assert!(info.signature.is_some());
        // Signed portion should NOT contain URL/Compression/FileSize
        assert!(!info.signed_portion.contains("URL:"));
        assert!(!info.signed_portion.contains("Compression:"));
        assert!(!info.signed_portion.contains("FileSize:"));
    }

    #[test]
    fn test_parse_multiple_references() {
        let data = "\
StorePath: /gnu/store/abc-test
NarHash: sha256:abc
NarSize: 100
References: /gnu/store/ref1 /gnu/store/ref2 /gnu/store/ref3
";
        let info = parse_narinfo(data).unwrap();
        assert_eq!(info.references.len(), 3);
        assert_eq!(info.references, vec!["/gnu/store/ref1", "/gnu/store/ref2", "/gnu/store/ref3"]);
    }

    #[test]
    fn test_parse_with_deriver() {
        let data = "\
StorePath: /gnu/store/abc-test
NarHash: sha256:abc
NarSize: 100
Deriver: /gnu/store/xyz-test.drv
";
        let info = parse_narinfo(data).unwrap();
        assert_eq!(info.deriver, Some("/gnu/store/xyz-test.drv".into()));
    }

    #[test]
    fn test_extract_hash_part() {
        assert_eq!(
            crate::daemon::extract_hash_part("/gnu/store/abc123def456ghi789jkl012mno345pq-foo-1.0")
                .unwrap(),
            "abc123def456ghi789jkl012mno345pq"
        );
    }
}
