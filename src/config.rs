use std::path::PathBuf;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub bootstrap_peers: Vec<String>,
    pub listen_addr: String,
    pub cache_dir: PathBuf,
    pub block_size: usize,
    pub request_timeout_secs: u64,
    pub stall_timeout_secs: u64,
    pub max_peers_per_download: usize,
    pub substitute_urls: Vec<String>,
    pub min_providers: usize,
    pub acl_path: PathBuf,
    pub max_total_peers: usize,
    pub connection_retries: u32,
    pub health_check_interval_secs: u64,
    pub reputation_ban_threshold: u32,
    pub reputation_prune_age_days: u64,
    pub dashboard_enabled: bool,
    pub dashboard_port: u16,
    pub dashboard_bind: String,
    pub tor_socks: Option<String>,
    pub tor_only: bool,
    pub socket_path: String,
    /// Store paths to seed on startup via `guix archive --export`.
    pub seed_paths: Vec<String>,
}

impl Config {
    pub fn load(
        cli_bootstrap: Option<String>,
        cli_listen: Option<String>,
        cli_cache: Option<String>,
        cli_substitute_urls: Option<String>,
    ) -> Self {
        let cache_dir = cli_cache.map(PathBuf::from).unwrap_or_else(dirs_cache_dir);

        Self {
            bootstrap_peers: cli_bootstrap
                .map(|s| s.split(',').map(str::to_string).collect())
                .unwrap_or_else(default_bootstrap_peers),
            listen_addr: cli_listen.unwrap_or_else(|| "/ip4/0.0.0.0/udp/6881/quic-v1".into()),
            cache_dir,
            block_size: 262144,
            request_timeout_secs: 30,
            stall_timeout_secs: 30,
            max_peers_per_download: 8,
            substitute_urls: cli_substitute_urls
                .map(|s| s.split(',').map(str::to_string).collect())
                .unwrap_or_else(default_substitute_urls),
            min_providers: 3,
            acl_path: PathBuf::from("/etc/guix/acl"),
            max_total_peers: 50,
            connection_retries: 3,
            health_check_interval_secs: 60,
            reputation_ban_threshold: 5,
            reputation_prune_age_days: 30,
            dashboard_enabled: false,
            dashboard_port: 3030,
            dashboard_bind: "127.0.0.1".into(),
            tor_socks: None,
            tor_only: false,
            socket_path: dirs_cache_dir().join("guix-p2p.sock").display().to_string(),
            seed_paths: vec![],
        }
    }
}

fn dirs_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(dir).join("guix-p2p")
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".cache/guix-p2p")
    }
}

fn default_bootstrap_peers() -> Vec<String> {
    vec![]
}

fn default_substitute_urls() -> Vec<String> {
    vec!["https://bordeaux.guix.gnu.org".into(), "https://ci.guix.gnu.org".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::load(None, None, Some("/tmp/guix-p2p-test".into()), None);
        assert_eq!(config.block_size, 262144);
        assert_eq!(config.max_peers_per_download, 8);
        assert_eq!(config.substitute_urls.len(), 2);
        assert_eq!(config.min_providers, 3);
        assert_eq!(config.stall_timeout_secs, 30);
        assert_eq!(config.dashboard_port, 3030);
        assert!(!config.dashboard_enabled);
    }
}
