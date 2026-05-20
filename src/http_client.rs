use crate::{
    bandwidth::BandwidthLimiter,
    config::Config,
    nar_hash,
    narinfo::{NarUrl, Narinfo, NarinfoCache, ParseError, load_acl_keys, verify_narinfo_signature},
};

#[derive(Debug, thiserror::Error)]
pub enum HttpClientError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("not found")]
    NotFound,
    #[error("bad signature")]
    BadSignature,
    #[error("decompression error: {0}")]
    Decompression(String),
    #[error("all HTTP nar candidates failed: {0}")]
    CandidatesFailed(String),
    #[error("{0}")]
    Other(String),
}

pub struct TorConfig {
    pub proxy_addr: String,
    pub only: bool,
}

#[derive(Debug)]
pub struct HttpNarDownload {
    pub data: Vec<u8>,
    pub source_url: String,
    pub verified: bool,
}

pub type HttpProgress<'a> = dyn FnMut(&str, u64, u64) + Send + 'a;

pub fn create_http_client(config: &Config) -> Result<reqwest::Client, HttpClientError> {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.request_timeout_secs));

    if let Some(ref proxy) = config.tor_socks {
        let proxy_url = format!("socks5h://{}", proxy);
        builder = builder.proxy(
            reqwest::Proxy::all(&proxy_url)
                .map_err(|e| HttpClientError::Other(format!("invalid SOCKS5 proxy: {}", e)))?,
        );

        if config.tor_only {
            builder = builder.no_proxy();
        }

        tracing::info!("HTTP traffic routed through SOCKS5 proxy: {}", proxy_url);
        if config.tor_only {
            tracing::info!("Tor-only mode: direct connections disabled");
        }
    }

    builder.build().map_err(HttpClientError::Http)
}

pub async fn fetch_narinfo(
    config: &Config,
    hash_part: &str,
    cache: &std::sync::Mutex<NarinfoCache>,
    client: &reqwest::Client,
) -> Result<Narinfo, HttpClientError> {
    if let Some(cached) = cache.lock().unwrap().get(hash_part) {
        tracing::debug!("Narinfo cache hit for {}", hash_part);
        return Ok(cached);
    }

    let keys = load_acl_keys(&config.acl_path)
        .map_err(|e| HttpClientError::Other(format!("failed to load ACL: {}", e)))?;
    let narinfo = fetch_verified_narinfo(config, hash_part, client, &keys).await?;

    cache.lock().unwrap().put(hash_part.to_string(), narinfo.clone());

    Ok(narinfo)
}

async fn fetch_verified_narinfo(
    config: &Config,
    hash_part: &str,
    client: &reqwest::Client,
    keys: &[ed25519_dalek::VerifyingKey],
) -> Result<Narinfo, HttpClientError> {
    let mut saw_bad_signature = false;
    for base_url in &config.substitute_urls {
        let url = format!("{}/{}.narinfo", base_url.trim_end_matches('/'), hash_part);
        tracing::debug!("Fetching narinfo from {}", url);
        match fetch_narinfo_from_url(&url, client).await {
            Ok(info) => {
                if verify_narinfo_signature(&info, keys) {
                    return Ok(info);
                }
                saw_bad_signature = true;
                tracing::warn!(
                    "Narinfo signature verification failed for {} from {}",
                    hash_part,
                    url
                );
                continue;
            },
            Err(HttpClientError::Http(e)) if e.status() == Some(reqwest::StatusCode::NOT_FOUND) => {
                continue;
            },
            Err(e) => {
                tracing::warn!("Failed to fetch narinfo from {}: {}", url, e);
                continue;
            },
        }
    }
    if saw_bad_signature {
        Err(HttpClientError::BadSignature)
    } else {
        Err(HttpClientError::NotFound)
    }
}

async fn fetch_narinfo_from_url(
    url: &str,
    client: &reqwest::Client,
) -> Result<Narinfo, HttpClientError> {
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(HttpClientError::Http(response.error_for_status().unwrap_err()));
    }
    let body = response.text().await?;
    Ok(crate::narinfo::parse_narinfo(&body)?)
}

pub fn first_nar_download_url(
    config: &Config,
    narinfo: &Narinfo,
) -> Result<Option<String>, HttpClientError> {
    let Some(download) = download_candidates(narinfo)?.into_iter().next() else {
        return Ok(None);
    };

    Ok(download_urls(config, &download.url).into_iter().next())
}

/// Download a compressed nar from a substitute server and decompress it.
/// Returns the raw (uncompressed) nar bytes and the source URL used.
/// Tries URLs in preference order: zstd > gzip > lzip > none.
pub async fn download_nar_http(
    config: &Config,
    narinfo: &Narinfo,
    client: &reqwest::Client,
    bandwidth_limiter: Option<&BandwidthLimiter>,
    mut progress: Option<&mut HttpProgress<'_>>,
) -> Result<HttpNarDownload, HttpClientError> {
    let downloads = download_candidates(narinfo)?;
    if downloads.is_empty() {
        return Err(HttpClientError::NotFound);
    }

    let mut last_error = None;
    let mut last_mismatched_download = None;
    for download in downloads {
        for full_url in download_urls(config, &download.url) {
            tracing::info!(
                "Downloading nar via HTTP: {} ({})",
                full_url,
                download.compression.as_str()
            );

            let response = match client.get(&full_url).send().await {
                Ok(response) => response,
                Err(err) => {
                    last_error = Some(HttpClientError::Http(err));
                    continue;
                },
            };

            if !response.status().is_success() {
                let err = response.error_for_status().unwrap_err();
                last_error = Some(HttpClientError::Http(err));
                continue;
            }

            let compressed =
                read_response_body(response, bandwidth_limiter, progress.as_deref_mut(), &full_url)
                    .await?;
            let nar_data = match download.compression.decompress(&compressed) {
                Ok(nar_data) => nar_data,
                Err(err) => {
                    last_error = Some(err);
                    continue;
                },
            };

            if !nar_hash_matches(&narinfo.nar_hash, &nar_data)? {
                tracing::warn!(
                    "HTTP nar hash mismatch for {}; trying next candidate if available",
                    full_url
                );
                last_mismatched_download =
                    Some(HttpNarDownload { data: nar_data, source_url: full_url, verified: false });
                continue;
            }

            tracing::info!(
                "HTTP nar download complete: {} bytes (compressed {} bytes)",
                nar_data.len(),
                compressed.len()
            );

            return Ok(HttpNarDownload { data: nar_data, source_url: full_url, verified: true });
        }
    }

    if let Some(download) = last_mismatched_download {
        return Ok(download);
    }

    match last_error {
        Some(err) => Err(HttpClientError::CandidatesFailed(err.to_string())),
        None => Err(HttpClientError::NotFound),
    }
}

fn nar_hash_matches(expected_nar_hash: &str, nar_data: &[u8]) -> Result<bool, HttpClientError> {
    use sha2::{Digest, Sha256};

    let Some(expected) = nar_hash::sha256_bytes(expected_nar_hash) else {
        return Err(HttpClientError::Other(format!("invalid nar hash: {}", expected_nar_hash)));
    };

    let actual = Sha256::digest(nar_data);
    Ok(actual.as_slice() == expected)
}

async fn read_response_body(
    response: reqwest::Response,
    bandwidth_limiter: Option<&BandwidthLimiter>,
    mut progress: Option<&mut HttpProgress<'_>>,
    source_url: &str,
) -> Result<Vec<u8>, HttpClientError> {
    use futures::StreamExt;

    let total = response.content_length().unwrap_or(0);
    let mut transferred = 0;
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if let Some(limiter) = bandwidth_limiter {
            limiter.wait_for_download(chunk.len() as u64).await;
        }
        transferred += chunk.len() as u64;
        if let Some(progress) = progress.as_mut() {
            progress(source_url, total, transferred);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn download_urls(config: &Config, nar_url: &str) -> Vec<String> {
    download_urls_from_bases(&config.substitute_urls, nar_url)
}

fn download_urls_from_bases(base_urls: &[String], nar_url: &str) -> Vec<String> {
    if nar_url.starts_with("http://") || nar_url.starts_with("https://") {
        return vec![nar_url.to_string()];
    }

    base_urls
        .iter()
        .map(|base_url| {
            format!("{}/{}", base_url.trim_end_matches('/'), nar_url.trim_start_matches('/'))
        })
        .collect()
}

/// Build nar download candidates based on compression preference.
/// Preference: zstd (best ratio + speed) > gzip (widely available) > lzip > none.
fn download_candidates(narinfo: &Narinfo) -> Result<Vec<NarDownload>, HttpClientError> {
    let mut candidates = Vec::new();
    let mut unsupported_compression = None;

    for pref in NarCompression::preference() {
        for url_entry in &narinfo.urls {
            match NarCompression::from_nar_url(url_entry) {
                Ok(compression) if compression == pref => {
                    candidates.push(NarDownload { url: url_entry.url.clone(), compression });
                },
                Ok(_) => {},
                Err(err) => {
                    unsupported_compression.get_or_insert(err);
                },
            }
        }
    }

    if candidates.is_empty()
        && let Some(err) = unsupported_compression
    {
        return Err(err);
    }

    Ok(candidates)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NarDownload {
    url: String,
    compression: NarCompression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NarCompression {
    Zstd,
    Gzip,
    Lzip,
    None,
}

impl NarCompression {
    fn preference() -> [Self; 4] {
        [Self::Zstd, Self::Gzip, Self::Lzip, Self::None]
    }

    fn from_nar_url(url: &NarUrl) -> Result<Self, HttpClientError> {
        Self::from_str(&url.compression)
    }

    fn from_str(value: &str) -> Result<Self, HttpClientError> {
        match value {
            "zstd" => Ok(Self::Zstd),
            "gzip" => Ok(Self::Gzip),
            "lzip" => Ok(Self::Lzip),
            "none" | "" => Ok(Self::None),
            other => {
                Err(HttpClientError::Decompression(format!("unsupported compression: {}", other)))
            },
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Zstd => "zstd",
            Self::Gzip => "gzip",
            Self::Lzip => "lzip",
            Self::None => "none",
        }
    }

    fn decompress(self, data: &[u8]) -> Result<Vec<u8>, HttpClientError> {
        match self {
            Self::Zstd => decompress_zstd(data),
            Self::Gzip => decompress_gzip(data),
            Self::Lzip => decompress_lzip(data),
            Self::None => Ok(data.to_vec()),
        }
    }
}

fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, HttpClientError> {
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(data);
    let mut output = Vec::with_capacity(data.len() * 4);
    decoder
        .read_to_end(&mut output)
        .map_err(|e| HttpClientError::Decompression(format!("gzip: {}", e)))?;
    Ok(output)
}

fn decompress_zstd(data: &[u8]) -> Result<Vec<u8>, HttpClientError> {
    let mut output = Vec::with_capacity(data.len() * 4);
    let mut decoder = zstd::Decoder::new(data)
        .map_err(|e| HttpClientError::Decompression(format!("zstd init: {}", e)))?;
    std::io::Read::read_to_end(&mut decoder, &mut output)
        .map_err(|e| HttpClientError::Decompression(format!("zstd: {}", e)))?;
    Ok(output)
}

fn decompress_lzip(data: &[u8]) -> Result<Vec<u8>, HttpClientError> {
    use std::io::Read;
    if !data.starts_with(b"LZIP") {
        return Err(HttpClientError::Decompression("lzip: bad magic".into()));
    }
    let mut decoder = lzma_rust2::LzipReader::new(data);
    let mut output = Vec::with_capacity(data.len() * 4);
    decoder
        .read_to_end(&mut output)
        .map_err(|e| HttpClientError::Decompression(format!("lzip: {}", e)))?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUIX_SIGNING_KEY_HEX: &str =
        "7D602902D3A2DBB83F8A0FB98602A754C5493B0B778C8D1DD4E0F41DE14DE34F";

    const VALID_HELLO_NARINFO: &str = "\
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
";

    fn narinfo_with_urls(urls: Vec<NarUrl>) -> Narinfo {
        Narinfo {
            store_path: "/gnu/store/example".into(),
            nar_hash: "sha256:example".into(),
            nar_size: 0,
            references: vec![],
            deriver: None,
            urls,
            signed_portion: String::new(),
            signature: None,
        }
    }

    fn nar_url(url: &str, compression: &str, file_size: u64) -> NarUrl {
        NarUrl { url: url.into(), compression: compression.into(), file_size }
    }

    fn narinfo_with_hash_and_urls(nar_hash: String, urls: Vec<NarUrl>) -> Narinfo {
        Narinfo { nar_hash, ..narinfo_with_urls(urls) }
    }

    fn acl_with_key(hex_key: &str) -> String {
        format!("(acl (entry (public-key (ecc (curve Ed25519) (q #{hex_key}#)))))")
    }

    #[test]
    fn download_urls_expands_relative_url_across_substitute_bases() {
        let base_urls = vec![
            "https://bordeaux.guix.gnu.org/".to_string(),
            "https://ci.guix.gnu.org".to_string(),
        ];

        let urls = download_urls_from_bases(&base_urls, "/nar/zstd/example");

        assert_eq!(
            urls,
            vec![
                "https://bordeaux.guix.gnu.org/nar/zstd/example",
                "https://ci.guix.gnu.org/nar/zstd/example",
            ]
        );
    }

    #[test]
    fn download_urls_keeps_absolute_url_as_single_candidate() {
        let base_urls = vec!["https://bordeaux.guix.gnu.org".to_string()];

        let urls = download_urls_from_bases(&base_urls, "https://mirror.example/nar/gzip/example");

        assert_eq!(urls, vec!["https://mirror.example/nar/gzip/example"]);
    }

    #[test]
    fn first_nar_download_url_reports_first_candidate_url() {
        let mut config = crate::config::Config::load(
            None,
            None,
            None,
            Some("/tmp/guix-p2p-test".into()),
            None,
            None,
        );
        config.substitute_urls =
            vec!["https://bordeaux.guix.gnu.org".into(), "https://ci.guix.gnu.org".into()];
        let narinfo = narinfo_with_urls(vec![
            nar_url("nar/gzip/example", "gzip", 10),
            nar_url("nar/zstd/example", "zstd", 20),
        ]);

        let url = first_nar_download_url(&config, &narinfo).unwrap().unwrap();

        assert_eq!(url, "https://bordeaux.guix.gnu.org/nar/zstd/example");
    }

    #[test]
    fn download_candidates_use_compression_preference() {
        let narinfo = narinfo_with_urls(vec![
            nar_url("nar/lzip/example", "lzip", 10),
            nar_url("nar/gzip/example", "gzip", 20),
            nar_url("nar/zstd/example", "zstd", 30),
        ]);

        let candidates = download_candidates(&narinfo).unwrap();

        assert_eq!(
            candidates,
            vec![
                NarDownload { url: "nar/zstd/example".into(), compression: NarCompression::Zstd },
                NarDownload { url: "nar/gzip/example".into(), compression: NarCompression::Gzip },
                NarDownload { url: "nar/lzip/example".into(), compression: NarCompression::Lzip },
            ]
        );
    }

    #[test]
    fn download_candidates_skip_unknown_when_supported_url_exists() {
        let narinfo = narinfo_with_urls(vec![
            nar_url("nar/br/example", "br", 10),
            nar_url("nar/gzip/example", "gzip", 20),
        ]);

        let candidates = download_candidates(&narinfo).unwrap();

        assert_eq!(
            candidates,
            vec![NarDownload { url: "nar/gzip/example".into(), compression: NarCompression::Gzip }]
        );
    }

    #[test]
    fn download_candidates_reject_all_unknown_compressions() {
        let narinfo = narinfo_with_urls(vec![nar_url("nar/br/example", "br", 10)]);

        let err = download_candidates(&narinfo).unwrap_err();

        assert!(err.to_string().contains("unsupported compression: br"));
    }

    #[test]
    fn compression_none_returns_original_data() {
        let data = b"raw nar bytes";

        let decompressed = NarCompression::None.decompress(data).unwrap();

        assert_eq!(decompressed, data);
    }

    #[test]
    fn nar_hash_matches_expected_sha256() {
        let hash = "sha256:4f78d3e7187277986632b4e63f366dae5812a000529278dd03f93f713517b394";

        assert!(nar_hash_matches(hash, b"raw nar bytes").unwrap());
        assert!(!nar_hash_matches(hash, b"different nar bytes").unwrap());
    }

    #[tokio::test]
    async fn download_nar_http_skips_mismatched_candidate() {
        use axum::{Router, routing::get};
        use sha2::{Digest, Sha256};

        let app = Router::new()
            .route("/bad", get(|| async { "bad nar" }))
            .route("/good", get(|| async { "good nar" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let good_hash = format!("sha256:{}", hex::encode(Sha256::digest(b"good nar")));
        let narinfo = narinfo_with_hash_and_urls(
            good_hash,
            vec![nar_url("bad", "none", 7), nar_url("good", "none", 8)],
        );
        let mut config = crate::config::Config::load(
            None,
            None,
            None,
            Some("/tmp/guix-p2p-test".into()),
            Some(base_url.clone()),
            None,
        );
        config.request_timeout_secs = 5;
        let client = create_http_client(&config).unwrap();

        let mut progress_events = Vec::new();
        let mut progress = |source_url: &str, total: u64, transferred: u64| {
            progress_events.push((source_url.to_string(), total, transferred));
        };

        let download =
            download_nar_http(&config, &narinfo, &client, None, Some(&mut progress)).await.unwrap();

        server.abort();
        assert_eq!(download.data, b"good nar");
        assert_eq!(download.source_url, format!("{}/good", base_url));
        assert!(download.verified);
        assert!(progress_events.contains(&(format!("{}/good", base_url), 8, 8)));
    }

    #[tokio::test]
    async fn fetch_narinfo_tries_next_substitute_after_bad_signature() {
        use axum::{Router, routing::get};

        let bad_narinfo = VALID_HELLO_NARINFO.replacen("NarSize: 282616", "NarSize: 282617", 1);
        let bad_app = Router::new().route(
            "/cs56i9digj9qg1bd383cmxc6xrfpdn9n.narinfo",
            get(move || async move { bad_narinfo }),
        );
        let bad_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bad_base = format!("http://{}", bad_listener.local_addr().unwrap());
        let bad_server = tokio::spawn(async move {
            axum::serve(bad_listener, bad_app).await.unwrap();
        });

        let good_app = Router::new().route(
            "/cs56i9digj9qg1bd383cmxc6xrfpdn9n.narinfo",
            get(|| async { VALID_HELLO_NARINFO }),
        );
        let good_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let good_base = format!("http://{}", good_listener.local_addr().unwrap());
        let good_server = tokio::spawn(async move {
            axum::serve(good_listener, good_app).await.unwrap();
        });

        let temp_dir = tempfile::tempdir().unwrap();
        let acl_path = temp_dir.path().join("acl");
        std::fs::write(&acl_path, acl_with_key(GUIX_SIGNING_KEY_HEX)).unwrap();
        let mut config = crate::config::Config::load(
            None,
            None,
            None,
            Some(temp_dir.path().join("cache").display().to_string()),
            None,
            None,
        );
        config.acl_path = acl_path;
        config.substitute_urls = vec![bad_base, good_base];
        config.request_timeout_secs = 5;
        let client = create_http_client(&config).unwrap();
        let cache = std::sync::Mutex::new(NarinfoCache::new(60));

        let narinfo = fetch_narinfo(&config, "cs56i9digj9qg1bd383cmxc6xrfpdn9n", &cache, &client)
            .await
            .unwrap();

        bad_server.abort();
        good_server.abort();
        assert_eq!(narinfo.nar_size, 282616);
    }

    #[tokio::test]
    async fn download_nar_http_tries_next_substitute_base() {
        use axum::{Router, http::StatusCode, routing::get};
        use sha2::{Digest, Sha256};

        let missing_app =
            Router::new().route("/nar/example", get(|| async { StatusCode::NOT_FOUND }));
        let missing_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let missing_base = format!("http://{}", missing_listener.local_addr().unwrap());
        let missing_server = tokio::spawn(async move {
            axum::serve(missing_listener, missing_app).await.unwrap();
        });

        let good_app = Router::new().route("/nar/example", get(|| async { "good nar" }));
        let good_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let good_base = format!("http://{}", good_listener.local_addr().unwrap());
        let good_server = tokio::spawn(async move {
            axum::serve(good_listener, good_app).await.unwrap();
        });

        let good_hash = format!("sha256:{}", hex::encode(Sha256::digest(b"good nar")));
        let narinfo =
            narinfo_with_hash_and_urls(good_hash, vec![nar_url("nar/example", "none", 8)]);
        let mut config = crate::config::Config::load(
            None,
            None,
            None,
            Some("/tmp/guix-p2p-test".into()),
            None,
            None,
        );
        config.substitute_urls = vec![missing_base, good_base.clone()];
        config.request_timeout_secs = 5;
        let client = create_http_client(&config).unwrap();

        let download = download_nar_http(&config, &narinfo, &client, None, None).await.unwrap();

        missing_server.abort();
        good_server.abort();
        assert_eq!(download.data, b"good nar");
        assert_eq!(download.source_url, format!("{}/nar/example", good_base));
        assert!(download.verified);
    }

    #[tokio::test]
    async fn download_nar_http_tries_next_compression_entry() {
        use axum::{Router, routing::get};
        use flate2::{Compression, write::GzEncoder};
        use sha2::{Digest, Sha256};
        use std::io::Write;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"good nar").unwrap();
        let gzip_bytes = encoder.finish().unwrap();

        let app = Router::new()
            .route("/bad-zstd", get(|| async { "not zstd" }))
            .route("/good-gzip", get(move || async move { gzip_bytes }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let good_hash = format!("sha256:{}", hex::encode(Sha256::digest(b"good nar")));
        let narinfo = narinfo_with_hash_and_urls(
            good_hash,
            vec![nar_url("bad-zstd", "zstd", 8), nar_url("good-gzip", "gzip", 8)],
        );
        let mut config = crate::config::Config::load(
            None,
            None,
            None,
            Some("/tmp/guix-p2p-test".into()),
            Some(base_url.clone()),
            None,
        );
        config.request_timeout_secs = 5;
        let client = create_http_client(&config).unwrap();

        let download = download_nar_http(&config, &narinfo, &client, None, None).await.unwrap();

        server.abort();
        assert_eq!(download.data, b"good nar");
        assert_eq!(download.source_url, format!("{}/good-gzip", base_url));
        assert!(download.verified);
    }

    #[tokio::test]
    async fn download_nar_http_returns_unverified_last_mismatch() {
        use axum::{Router, routing::get};
        use sha2::{Digest, Sha256};

        let app = Router::new().route("/bad", get(|| async { "bad nar" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let good_hash = format!("sha256:{}", hex::encode(Sha256::digest(b"good nar")));
        let narinfo = narinfo_with_hash_and_urls(good_hash, vec![nar_url("bad", "none", 7)]);
        let mut config = crate::config::Config::load(
            None,
            None,
            None,
            Some("/tmp/guix-p2p-test".into()),
            Some(base_url.clone()),
            None,
        );
        config.request_timeout_secs = 5;
        let client = create_http_client(&config).unwrap();

        let download = download_nar_http(&config, &narinfo, &client, None, None).await.unwrap();

        server.abort();
        assert_eq!(download.data, b"bad nar");
        assert_eq!(download.source_url, format!("{}/bad", base_url));
        assert!(!download.verified);
    }

    #[tokio::test]
    async fn download_nar_http_reports_all_candidates_failed() {
        use axum::{Router, routing::get};

        let app = Router::new().route("/bad-zstd", get(|| async { "not zstd" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let narinfo = narinfo_with_hash_and_urls(
            "sha256:4f78d3e7187277986632b4e63f366dae5812a000529278dd03f93f713517b394".into(),
            vec![nar_url("bad-zstd", "zstd", 8)],
        );
        let mut config = crate::config::Config::load(
            None,
            None,
            None,
            Some("/tmp/guix-p2p-test".into()),
            Some(base_url),
            None,
        );
        config.request_timeout_secs = 5;
        let client = create_http_client(&config).unwrap();

        let err = download_nar_http(&config, &narinfo, &client, None, None).await.unwrap_err();

        server.abort();
        assert!(matches!(err, HttpClientError::CandidatesFailed(_)));
    }

    #[test]
    fn malformed_lzip_returns_decompression_error() {
        let err = NarCompression::Lzip.decompress(b"not lzip").unwrap_err();

        assert!(err.to_string().contains("lzip"));
    }

    #[test]
    fn decompresses_lzip_data() {
        let compressed = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            "TFpJUAEMADOdSVdY8uL3xFsdz3xwxWYVvPpg1Ka6hh+L//++hUAAMU+W7hYAAAAAAAAAOwAAAAAAAAA=",
        )
        .unwrap();

        let decompressed = decompress_lzip(&compressed).unwrap();

        assert_eq!(decompressed, b"guix-p2p lzip fixture\n");
    }
}
