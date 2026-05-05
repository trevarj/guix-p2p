use crate::{
    config::Config,
    narinfo::{Narinfo, NarinfoCache, ParseError, load_acl_keys, verify_narinfo_signature},
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
