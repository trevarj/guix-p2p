use std::time::Duration;

use leaky_bucket::RateLimiter;

#[derive(Debug, Clone, Default)]
pub struct BandwidthConfig {
    pub upload_limit_bytes_per_sec: Option<u64>,
    pub download_limit_bytes_per_sec: Option<u64>,
}

#[derive(Debug)]
pub struct BandwidthLimiter {
    upload: Option<RateLimiter>,
    download: Option<RateLimiter>,
}

impl BandwidthLimiter {
    pub fn new(config: BandwidthConfig) -> Self {
        BandwidthLimiter {
            upload: rate_limiter(config.upload_limit_bytes_per_sec),
            download: rate_limiter(config.download_limit_bytes_per_sec),
        }
    }

    pub async fn wait_for_upload(&self, bytes: u64) {
        consume(&self.upload, bytes).await;
    }

    pub async fn wait_for_download(&self, bytes: u64) {
        consume(&self.download, bytes).await;
    }
}

fn rate_limiter(bytes_per_sec: Option<u64>) -> Option<RateLimiter> {
    let rate = bytes_per_sec?;
    if rate == 0 {
        return None;
    }

    let rate = usize::try_from(rate).unwrap_or(usize::MAX);
    Some(
        RateLimiter::builder()
            .max(rate)
            .initial(rate)
            .refill(rate)
            .interval(Duration::from_secs(1))
            .build(),
    )
}

async fn consume(limiter: &Option<RateLimiter>, bytes: u64) {
    let Some(limiter) = limiter else {
        return;
    };

    let mut remaining = usize::try_from(bytes).unwrap_or(usize::MAX);
    let max = limiter.max().max(1);
    while remaining > 0 {
        let chunk = remaining.min(max);
        limiter.acquire(chunk).await;
        remaining -= chunk;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn test_no_limit_is_noop() {
        let limiter = BandwidthLimiter::new(BandwidthConfig::default());
        let start = Instant::now();
        limiter.wait_for_download(1024 * 1024).await;
        assert!(start.elapsed() < Duration::from_millis(10));
    }

    #[tokio::test]
    async fn test_rate_limited() {
        let config =
            BandwidthConfig { download_limit_bytes_per_sec: Some(100_000), ..Default::default() };
        let limiter = BandwidthLimiter::new(config);

        let start = Instant::now();
        limiter.wait_for_download(1_000_000).await;
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_millis(9_000));
        assert!(elapsed < Duration::from_millis(12_000));
    }

    #[tokio::test]
    async fn test_upload_rate_limited() {
        let config =
            BandwidthConfig { upload_limit_bytes_per_sec: Some(100_000), ..Default::default() };
        let limiter = BandwidthLimiter::new(config);

        let start = Instant::now();
        limiter.wait_for_upload(1_000_000).await;
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_millis(9_000));
        assert!(elapsed < Duration::from_millis(12_000));
    }

    #[tokio::test]
    async fn concurrent_uploads_share_one_limit() {
        let config =
            BandwidthConfig { upload_limit_bytes_per_sec: Some(100_000), ..Default::default() };
        let limiter = std::sync::Arc::new(BandwidthLimiter::new(config));

        let start = Instant::now();
        let first = tokio::spawn({
            let limiter = limiter.clone();
            async move { limiter.wait_for_upload(100_000).await }
        });
        let second = tokio::spawn({
            let limiter = limiter.clone();
            async move { limiter.wait_for_upload(100_000).await }
        });

        first.await.unwrap();
        second.await.unwrap();
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_millis(900));
        assert!(elapsed < Duration::from_millis(1_500));
    }
}
