use std::{collections::HashMap, ffi::CString, ptr, slice, sync::Once};

use ed25519_dalek::VerifyingKey;
use libgcrypt_sys::{
    gcry_check_version, gcry_pk_verify, gcry_sexp_find_token, gcry_sexp_new, gcry_sexp_nth_data,
    gcry_sexp_release, gcry_sexp_t,
};
use sha2::{Digest, Sha256};

static GCRYPT_INIT: Once = Once::new();

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
        let (expiry, info) = self.entries.get(hash_part)?;
        if tokio::time::Instant::now() >= *expiry {
            return None;
        }
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
        (above.to_string(), Some(below.to_string()))
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

/// Verify the narinfo's Guix SPKI signature against a list of authorized keys.
/// Returns true if an authorized key made a valid signature.
pub fn verify_narinfo_signature(narinfo: &Narinfo, keys: &[VerifyingKey]) -> bool {
    init_gcrypt();

    let sig_str = match &narinfo.signature {
        Some(s) => s,
        None => return false,
    };

    // Parse "1;hostname;base64-signature"
    let parts: Vec<&str> = sig_str.splitn(3, ';').collect();
    if parts.len() < 3 {
        return false;
    }
    if parts[0] != "1" {
        return false;
    }

    let b64 = parts[2];
    let sig_bytes = match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let signature = match GcryptSexp::new(&sig_bytes) {
        Some(signature) => signature,
        None => return false,
    };

    let hash = match signature.find_token("hash") {
        Some(hash) => hash,
        None => return false,
    };
    if hash.data_at(1).as_deref() != Some(b"sha256") {
        return false;
    }
    let signed_hash = match hash.data_at(2) {
        Some(bytes) => bytes,
        None => return false,
    };
    let expected_hash = Sha256::digest(narinfo.signed_portion.as_bytes());
    if signed_hash.as_slice() != expected_hash.as_slice() {
        return false;
    }

    let q = match signature.find_token("q").and_then(|q| q.data_at(1)) {
        Some(bytes) if bytes.len() == 32 => bytes,
        _ => return false,
    };
    if !keys.iter().any(|key| key.to_bytes().as_slice() == q.as_slice()) {
        return false;
    }

    signature.verify()
}

fn init_gcrypt() {
    GCRYPT_INIT.call_once(|| unsafe {
        gcry_check_version(ptr::null());
    });
}

impl GcryptSexp {
    fn new(bytes: &[u8]) -> Option<Self> {
        let mut raw = ptr::null_mut();
        let err = unsafe { gcry_sexp_new(&mut raw, bytes.as_ptr().cast(), bytes.len(), 1) };
        (err == 0 && !raw.is_null()).then_some(Self { raw })
    }

    fn find_token(&self, token: &str) -> Option<Self> {
        let token = CString::new(token).ok()?;
        let raw = unsafe { gcry_sexp_find_token(self.raw, token.as_ptr(), 0) };
        (!raw.is_null()).then_some(Self { raw })
    }

    fn data_at(&self, index: i32) -> Option<Vec<u8>> {
        let mut len = 0usize;
        let data = unsafe { gcry_sexp_nth_data(self.raw, index, &mut len) };
        (!data.is_null()).then(|| unsafe { slice::from_raw_parts(data.cast(), len).to_vec() })
    }

    fn verify(&self) -> bool {
        let data = match self.find_token("data") {
            Some(data) => data,
            None => return false,
        };
        let sig_val = match self.find_token("sig-val") {
            Some(sig_val) => sig_val,
            None => return false,
        };
        let public_key = match self.find_token("public-key") {
            Some(public_key) => public_key,
            None => return false,
        };

        unsafe { gcry_pk_verify(sig_val.raw, data.raw, public_key.raw) == 0 }
    }
}

struct GcryptSexp {
    raw: gcry_sexp_t,
}

impl Drop for GcryptSexp {
    fn drop(&mut self) {
        unsafe { gcry_sexp_release(self.raw) };
    }
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
    fn test_verify_real_guix_narinfo_signature() {
        let data = "\
StorePath: /gnu/store/cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2
NarHash: sha256:0qhasy0w9w9mfv0vacgzymxl4nww8cslyza5x2ci42v7i2b13lyl
NarSize: 282616
References: cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2 m2vhzr0dy352cn59sgcklcaykprrr4j6-gcc-14.3.0-lib yj053cys0724p7vs9kir808x7fivz17m-glibc-2.41
System: x86_64-linux
Deriver: rxw8g87bf61bwbagfq38sp5xwy28jb5d-hello-2.12.2.drv
Signature: 1;bayfront;KHNpZ25hdHVyZSAKIChkYXRhIAogIChmbGFncyByZmM2OTc5KQogIChoYXNoIHNoYTI1NiAjMENDQjE0QjFFNkZFQUI4OTIyRjVGN0NFQjQ3QzRENUQ5N0E4QTFFNzk3RkIyM0RDREY5N0QyQkRFODA4MjYyQSMpCiAgKQogKHNpZy12YWwgCiAgKGVjZHNhIAogICAociAjMEJBNkY2ODkzQjhEQThCQ0ZCNUMxNDk2QTUwMDA4MTIzNUUyMjFCQkU4RDFCNUJBOEQ3NTk1REUyNkYxNUYxNCMpCiAgIChzICMwM0RCOUQ0MzA1QUEwRjQ3N0NCMDM4MkEyMzJBNzFGNUMyQkFEOEJBRjEwQzJGNURCMUM0NTZFNTE1MjA5RTIxIykKICAgKQogICkKIChwdWJsaWMta2V5IAogIChlY2MgCiAgIChjdXJ2ZSBFZDI1NTE5KQogICAocSAjN0Q2MDI5MDJEM0EyREJCODNGOEEwRkI5ODYwMkE3NTRDNTQ5M0IwQjc3OEM4RDFERDRFMEY0MURFMTRERTM0RiMpCiAgICkKICApCiApCg==
URL: nar/lzip/cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2
Compression: lzip
FileSize: 68076
URL: nar/zstd/cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2
Compression: zstd
FileSize: 73413
";
        let info = parse_narinfo(data).unwrap();
        let key = VerifyingKey::from_bytes(
            &hex::decode("7D602902D3A2DBB83F8A0FB98602A754C5493B0B778C8D1DD4E0F41DE14DE34F")
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .unwrap();

        assert!(verify_narinfo_signature(&info, &[key]));
    }

    #[test]
    fn test_verify_real_guix_narinfo_signature_rejects_tampered_signed_data() {
        let data = "\
StorePath: /gnu/store/cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2
NarHash: sha256:1qhasy0w9w9mfv0vacgzymxl4nww8cslyza5x2ci42v7i2b13lyl
NarSize: 282616
References: cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2 m2vhzr0dy352cn59sgcklcaykprrr4j6-gcc-14.3.0-lib yj053cys0724p7vs9kir808x7fivz17m-glibc-2.41
System: x86_64-linux
Deriver: rxw8g87bf61bwbagfq38sp5xwy28jb5d-hello-2.12.2.drv
Signature: 1;bayfront;KHNpZ25hdHVyZSAKIChkYXRhIAogIChmbGFncyByZmM2OTc5KQogIChoYXNoIHNoYTI1NiAjMENDQjE0QjFFNkZFQUI4OTIyRjVGN0NFQjQ3QzRENUQ5N0E4QTFFNzk3RkIyM0RDREY5N0QyQkRFODA4MjYyQSMpCiAgKQogKHNpZy12YWwgCiAgKGVjZHNhIAogICAociAjMEJBNkY2ODkzQjhEQThCQ0ZCNUMxNDk2QTUwMDA4MTIzNUUyMjFCQkU4RDFCNUJBOEQ3NTk1REUyNkYxNUYxNCMpCiAgIChzICMwM0RCOUQ0MzA1QUEwRjQ3N0NCMDM4MkEyMzJBNzFGNUMyQkFEOEJBRjEwQzJGNURCMUM0NTZFNTE1MjA5RTIxIykKICAgKQogICkKIChwdWJsaWMta2V5IAogIChlY2MgCiAgIChjdXJ2ZSBFZDI1NTE5KQogICAocSAjN0Q2MDI5MDJEM0EyREJCODNGOEEwRkI5ODYwMkE3NTRDNTQ5M0IwQjc3OEM4RDFERDRFMEY0MURFMTRERTM0RiMpCiAgICkKICApCiApCg==
";
        let info = parse_narinfo(data).unwrap();
        let key = VerifyingKey::from_bytes(
            &hex::decode("7D602902D3A2DBB83F8A0FB98602A754C5493B0B778C8D1DD4E0F41DE14DE34F")
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .unwrap();

        assert!(!verify_narinfo_signature(&info, &[key]));
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
            crate::store_path::hash_part("/gnu/store/abc123def456ghi789jkl012mno345pq-foo-1.0")
                .unwrap(),
            "abc123def456ghi789jkl012mno345pq"
        );
    }
}
