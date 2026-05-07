use std::path::PathBuf;

/// Policy controlling how substitutes are sourced when both P2P and HTTP are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SubstitutePolicy {
    /// Only use P2P swarm. Return not-found if no peers have the nar.
    P2pOnly,
    /// Try P2P first. Fall back to HTTP nar download if swarm fails.
    #[default]
    P2pFirst,
    /// Try HTTP first. Fall back to P2P if HTTP fails or is slow.
    HttpFirst,
}

impl std::fmt::Display for SubstitutePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubstitutePolicy::P2pOnly => write!(f, "p2p-only"),
            SubstitutePolicy::P2pFirst => write!(f, "p2p-first"),
            SubstitutePolicy::HttpFirst => write!(f, "http-first"),
        }
    }
}

impl std::str::FromStr for SubstitutePolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "p2p-only" => Ok(SubstitutePolicy::P2pOnly),
            "p2p-first" => Ok(SubstitutePolicy::P2pFirst),
            "http-first" => Ok(SubstitutePolicy::HttpFirst),
            _ => Err(format!(
                "unknown substitute policy '{}'; expected p2p-only, p2p-first, or http-first",
                s
            )),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, Default)]
#[serde(default)]
struct ConfigFile {
    bootstrap_peers: Option<String>,
    listen_addr: Option<String>,
    cache_dir: Option<String>,
    substitute_urls: Option<String>,
    substitute_policy: Option<SubstitutePolicy>,
    block_size: Option<usize>,
    request_timeout_secs: Option<u64>,
    stall_timeout_secs: Option<u64>,
    max_peers_per_download: Option<usize>,
    min_providers: Option<usize>,
    max_total_peers: Option<usize>,
    connection_retries: Option<u32>,
    health_check_interval_secs: Option<u64>,
    reputation_ban_threshold: Option<u32>,
    acl_path: Option<String>,
    dashboard_enabled: Option<bool>,
    dashboard_port: Option<u16>,
    dashboard_bind: Option<String>,
    tor_socks: Option<String>,
    tor_only: Option<bool>,
    socket_path: Option<String>,
    seed_paths: Option<Vec<String>>,
}

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
    pub substitute_policy: SubstitutePolicy,
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
        cli_policy: Option<SubstitutePolicy>,
    ) -> Self {
        let file = load_config_file();
        let cache_dir = cli_cache
            .as_ref()
            .or(file.cache_dir.as_ref())
            .map(PathBuf::from)
            .unwrap_or_else(dirs_cache_dir);

        Self {
            bootstrap_peers: cli_bootstrap
                .map(|s| s.split(',').map(str::to_string).collect())
                .or_else(|| {
                    file.bootstrap_peers
                        .as_ref()
                        .map(|s| s.split(',').map(str::to_string).collect())
                })
                .unwrap_or_else(default_bootstrap_peers),
            listen_addr: cli_listen
                .or(file.listen_addr)
                .unwrap_or_else(|| "/ip4/0.0.0.0/udp/6881/quic-v1".into()),
            cache_dir: cache_dir.clone(),
            block_size: file.block_size.unwrap_or(262144),
            request_timeout_secs: file.request_timeout_secs.unwrap_or(30),
            stall_timeout_secs: file.stall_timeout_secs.unwrap_or(30),
            max_peers_per_download: file.max_peers_per_download.unwrap_or(8),
            substitute_urls: cli_substitute_urls
                .map(|s| s.split(',').map(str::to_string).collect())
                .or_else(|| {
                    file.substitute_urls
                        .as_ref()
                        .map(|s| s.split(',').map(str::to_string).collect())
                })
                .unwrap_or_else(default_substitute_urls),
            substitute_policy: cli_policy.or(file.substitute_policy).unwrap_or_default(),
            min_providers: file.min_providers.unwrap_or(3),
            acl_path: file
                .acl_path
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/etc/guix/acl")),
            max_total_peers: file.max_total_peers.unwrap_or(50),
            connection_retries: file.connection_retries.unwrap_or(3),
            health_check_interval_secs: file.health_check_interval_secs.unwrap_or(60),
            reputation_ban_threshold: file.reputation_ban_threshold.unwrap_or(5),
            reputation_prune_age_days: 30,
            dashboard_enabled: file.dashboard_enabled.unwrap_or(false),
            dashboard_port: file.dashboard_port.unwrap_or(3030),
            dashboard_bind: file.dashboard_bind.unwrap_or_else(|| "127.0.0.1".into()),
            tor_socks: file.tor_socks,
            tor_only: file.tor_only.unwrap_or(false),
            socket_path: file
                .socket_path
                .unwrap_or_else(|| cache_dir.join("guix-p2p.sock").display().to_string()),
            seed_paths: file.seed_paths.unwrap_or_default(),
        }
    }
}

fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(dir).join("guix-p2p")
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".config/guix-p2p")
    }
}

fn load_config_file() -> ConfigFile {
    let path = config_dir().join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(content) => match toml::from_str(&content) {
            Ok(cfg) => {
                tracing::info!("loaded config from {}", path.display());
                cfg
            },
            Err(e) => {
                tracing::warn!("failed to parse config file {}: {}", path.display(), e);
                ConfigFile::default()
            },
        },
        Err(_) => ConfigFile::default(),
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
        let config = Config::load(None, None, Some("/tmp/guix-p2p-test".into()), None, None);
        assert_eq!(config.block_size, 262144);
        assert_eq!(config.max_peers_per_download, 8);
        assert_eq!(config.substitute_urls.len(), 2);
        assert_eq!(config.min_providers, 3);
        assert_eq!(config.stall_timeout_secs, 30);
        assert_eq!(config.dashboard_port, 3030);
        assert!(!config.dashboard_enabled);
        assert_eq!(config.substitute_policy, SubstitutePolicy::P2pFirst);
    }

    #[test]
    fn test_policy_from_str() {
        assert_eq!("p2p-only".parse(), Ok(SubstitutePolicy::P2pOnly));
        assert_eq!("p2p-first".parse(), Ok(SubstitutePolicy::P2pFirst));
        assert_eq!("http-first".parse(), Ok(SubstitutePolicy::HttpFirst));
        assert!("invalid".parse::<SubstitutePolicy>().is_err());
    }

    #[test]
    fn test_policy_display_roundtrip() {
        for policy in
            [SubstitutePolicy::P2pOnly, SubstitutePolicy::P2pFirst, SubstitutePolicy::HttpFirst]
        {
            let s = policy.to_string();
            assert_eq!(s.parse::<SubstitutePolicy>().unwrap(), policy);
        }
    }
}
