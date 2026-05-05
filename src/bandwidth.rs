use std::time::{Duration, Instant};

use tokio::sync::Mutex;

#[derive(Debug, Clone, Default)]
pub struct BandwidthConfig {
    pub upload_limit_bytes_per_sec: Option<u64>,
    pub download_limit_bytes_per_sec: Option<u64>,
}

#[derive(Debug)]
pub struct BandwidthLimiter {
    config: BandwidthConfig,
    state: Mutex<LimiterState>,
}

#[derive(Debug)]
struct LimiterState {
    upload_tokens: f64,
    download_tokens: f64,
    last_refill: Instant,
}

impl BandwidthLimiter {
    pub fn new(config: BandwidthConfig) -> Self {
        let caps = (
            config.upload_limit_bytes_per_sec.unwrap_or(u64::MAX) as f64,
            config.download_limit_bytes_per_sec.unwrap_or(u64::MAX) as f64,
        );

        BandwidthLimiter {
            config,
            state: Mutex::new(LimiterState {
                upload_tokens: caps.0,
                download_tokens: caps.1,
                last_refill: Instant::now(),
            }),
        }
    }

    pub async fn wait_for_upload(&self, bytes: u64) {
        self.consume(bytes, Direction::Upload).await;
    }

    pub async fn wait_for_download(&self, bytes: u64) {
        self.consume(bytes, Direction::Download).await;
    }

    async fn consume(&self, bytes: u64, direction: Direction) {
        let cap = match direction {
            Direction::Upload => self.config.upload_limit_bytes_per_sec,
            Direction::Download => self.config.download_limit_bytes_per_sec,
        };

        if cap.is_none() || cap == Some(0) {
            return;
        }

        let rate = cap.unwrap() as f64;
        let mut state = self.state.lock().await;
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();

        state.upload_tokens = (state.upload_tokens + elapsed * rate).min(rate);
        state.download_tokens = (state.download_tokens + elapsed * rate).min(rate);
        state.last_refill = now;

        let tokens = match direction {
            Direction::Upload => &mut state.upload_tokens,
            Direction::Download => &mut state.download_tokens,
        };

        let needed = bytes as f64;
        if *tokens >= needed {
            *tokens -= needed;
            return;
        }

        let deficit = needed - *tokens;
        *tokens = 0.0;
        drop(state);

        let wait = Duration::from_secs_f64(deficit / rate);
        tokio::time::sleep(wait).await;
    }
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Upload,
    Download,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
