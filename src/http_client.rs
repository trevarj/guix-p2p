use crate::{
    config::Config,
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
    #[error("{0}")]
    Other(String),
}

pub struct TorConfig {
    pub proxy_addr: String,
    pub only: bool,
}

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

    let narinfo = fetch_narinfo_raw(config, hash_part, client).await?;

    let keys = load_acl_keys(&config.acl_path)
        .map_err(|e| HttpClientError::Other(format!("failed to load ACL: {}", e)))?;

    if !verify_narinfo_signature(&narinfo, &keys) {
        tracing::warn!("Narinfo signature verification failed for {}", hash_part);
        return Err(HttpClientError::BadSignature);
    }

    cache.lock().unwrap().put(hash_part.to_string(), narinfo.clone());

    Ok(narinfo)
}

async fn fetch_narinfo_raw(
    config: &Config,
    hash_part: &str,
    client: &reqwest::Client,
) -> Result<Narinfo, HttpClientError> {
    for base_url in &config.substitute_urls {
        let url = format!("{}/{}.narinfo", base_url.trim_end_matches('/'), hash_part);
        tracing::debug!("Fetching narinfo from {}", url);
        match fetch_narinfo_from_url(&url, client).await {
            Ok(info) => return Ok(info),
            Err(HttpClientError::Http(e)) if e.status() == Some(reqwest::StatusCode::NOT_FOUND) => {
                continue;
            },
            Err(e) => {
                tracing::warn!("Failed to fetch narinfo from {}: {}", url, e);
                continue;
            },
        }
    }
    Err(HttpClientError::NotFound)
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

/// Download a compressed nar from a substitute server and decompress it.
/// Returns the raw (uncompressed) nar bytes.
/// Tries URLs in preference order: zstd > gzip > lzip > none.
pub async fn download_nar_http(
    config: &Config,
    narinfo: &Narinfo,
    client: &reqwest::Client,
) -> Result<Vec<u8>, HttpClientError> {
    let best = choose_best_url(narinfo)?;
    let download = match best {
        Some(download) => download,
        None => return Err(HttpClientError::NotFound),
    };

    let mut last_error = None;
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

        let compressed = response.bytes().await?;
        let nar_data = download.compression.decompress(&compressed)?;

        tracing::info!(
            "HTTP nar download complete: {} bytes (compressed {} bytes)",
            nar_data.len(),
            compressed.len()
        );

        return Ok(nar_data);
    }

    Err(last_error.unwrap_or(HttpClientError::NotFound))
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

/// Choose the best nar URL based on compression and file size.
/// Preference: zstd (best ratio + speed) > gzip (widely available) > lzip > none.
fn choose_best_url(narinfo: &Narinfo) -> Result<Option<NarDownload>, HttpClientError> {
    let mut unsupported_compression = None;

    for pref in NarCompression::preference() {
        for url_entry in &narinfo.urls {
            match NarCompression::from_nar_url(url_entry) {
                Ok(compression) if compression == pref => {
                    return Ok(Some(NarDownload { url: url_entry.url.clone(), compression }));
                },
                Ok(_) => {},
                Err(err) => {
                    unsupported_compression.get_or_insert(err);
                },
            }
        }
    }

    match unsupported_compression {
        Some(err) => Err(err),
        None => Ok(None),
    }
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
    fn choose_best_url_uses_compression_preference() {
        let narinfo = narinfo_with_urls(vec![
            nar_url("nar/lzip/example", "lzip", 10),
            nar_url("nar/gzip/example", "gzip", 20),
            nar_url("nar/zstd/example", "zstd", 30),
        ]);

        let selected = choose_best_url(&narinfo).unwrap().unwrap();

        assert_eq!(selected.url, "nar/zstd/example");
        assert_eq!(selected.compression, NarCompression::Zstd);
    }

    #[test]
    fn choose_best_url_skips_unknown_when_supported_url_exists() {
        let narinfo = narinfo_with_urls(vec![
            nar_url("nar/br/example", "br", 10),
            nar_url("nar/gzip/example", "gzip", 20),
        ]);

        let selected = choose_best_url(&narinfo).unwrap().unwrap();

        assert_eq!(selected.url, "nar/gzip/example");
        assert_eq!(selected.compression, NarCompression::Gzip);
    }

    #[test]
    fn choose_best_url_rejects_all_unknown_compressions() {
        let narinfo = narinfo_with_urls(vec![nar_url("nar/br/example", "br", 10)]);

        let err = choose_best_url(&narinfo).unwrap_err();

        assert!(err.to_string().contains("unsupported compression: br"));
    }

    #[test]
    fn compression_none_returns_original_data() {
        let data = b"raw nar bytes";

        let decompressed = NarCompression::None.decompress(data).unwrap();

        assert_eq!(decompressed, data);
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
