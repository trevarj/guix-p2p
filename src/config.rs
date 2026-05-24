use std::path::{Path, PathBuf};

/// Policy controlling how substitutes are sourced when both P2P and HTTP are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SubstitutePolicy {
    /// Only use P2P swarm. Return not-found if no peers have the nar.
    P2pOnly,
    /// Try P2P first. Fall back to HTTP nar download if swarm fails.
    P2pFirst,
    /// Try HTTP first. Fall back to P2P if HTTP fails or is slow.
    #[default]
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

/// Which successful substitute downloads should be cached for later P2P seeding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AutoSeedDownloads {
    /// Never cache downloads automatically.
    Off,
    /// Cache only downloads that came from P2P peers.
    #[default]
    P2p,
    /// Cache downloads from P2P peers and HTTP substitute servers.
    All,
}

impl AutoSeedDownloads {
    pub fn should_seed_p2p(self) -> bool {
        matches!(self, AutoSeedDownloads::P2p | AutoSeedDownloads::All)
    }

    pub fn should_seed_http(self) -> bool {
        matches!(self, AutoSeedDownloads::All)
    }
}

impl std::fmt::Display for AutoSeedDownloads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutoSeedDownloads::Off => write!(f, "off"),
            AutoSeedDownloads::P2p => write!(f, "p2p"),
            AutoSeedDownloads::All => write!(f, "all"),
        }
    }
}

impl std::str::FromStr for AutoSeedDownloads {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "off" => Ok(AutoSeedDownloads::Off),
            "p2p" => Ok(AutoSeedDownloads::P2p),
            "all" => Ok(AutoSeedDownloads::All),
            _ => Err(format!("unknown auto-seed mode '{}'; expected off, p2p, or all", s)),
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(untagged)]
enum AutoSeedDownloadsConfig {
    Mode(AutoSeedDownloads),
    Enabled(bool),
}

impl AutoSeedDownloadsConfig {
    fn into_mode(self) -> AutoSeedDownloads {
        match self {
            AutoSeedDownloadsConfig::Mode(mode) => mode,
            AutoSeedDownloadsConfig::Enabled(true) => AutoSeedDownloads::All,
            AutoSeedDownloadsConfig::Enabled(false) => AutoSeedDownloads::Off,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, Default)]
#[serde(default)]
struct ConfigFile {
    bootstrap_peers: Option<String>,
    enable_default_bootstrap_peers: Option<bool>,
    peer_store_enabled: Option<bool>,
    peer_store_max_entries: Option<usize>,
    external_addresses: Option<String>,
    listen_addr: Option<String>,
    cache_dir: Option<String>,
    substitute_urls: Option<String>,
    substitute_policy: Option<SubstitutePolicy>,
    block_size: Option<usize>,
    request_timeout_secs: Option<u64>,
    stall_timeout_secs: Option<u64>,
    max_peers_per_download: Option<usize>,
    max_in_flight_blocks_per_peer: Option<usize>,
    max_upload_rate_kbps: Option<u64>,
    max_download_rate_kbps: Option<u64>,
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
    auto_seed_downloads: Option<AutoSeedDownloadsConfig>,
    local_narinfo_path: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub bootstrap_peers: Vec<String>,
    pub enable_default_bootstrap_peers: bool,
    pub peer_store_enabled: bool,
    pub peer_store_max_entries: usize,
    pub external_addresses: Vec<String>,
    pub listen_addr: String,
    pub cache_dir: PathBuf,
    pub block_size: usize,
    pub request_timeout_secs: u64,
    pub stall_timeout_secs: u64,
    pub max_peers_per_download: usize,
    pub max_in_flight_blocks_per_peer: usize,
    /// Optional upload cap for serving P2P blocks, in KiB/s.
    pub max_upload_rate_kbps: Option<u64>,
    /// Optional download cap for HTTP fallback responses, in KiB/s.
    pub max_download_rate_kbps: Option<u64>,
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
    /// Which successful substitute downloads to cache and announce for re-seeding.
    pub auto_seed_downloads: AutoSeedDownloads,
    /// Optional JSON metadata file for offline narinfo lookups.
    pub local_narinfo_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct InitConfigOptions {
    pub bootstrap_peers: Option<String>,
    pub external_addresses: Option<String>,
    pub listen_addr: Option<String>,
    pub cache_dir: Option<String>,
    pub substitute_urls: Option<String>,
    pub substitute_policy: Option<SubstitutePolicy>,
    pub socket_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitConfigResult {
    Created,
    AlreadyExists,
}

impl Config {
    pub fn load(
        cli_bootstrap: Option<String>,
        cli_external_addresses: Option<String>,
        cli_listen: Option<String>,
        cli_cache: Option<String>,
        cli_substitute_urls: Option<String>,
        cli_policy: Option<SubstitutePolicy>,
    ) -> Self {
        let file = load_config_file();
        Self::load_from_file(
            file,
            cli_bootstrap,
            cli_external_addresses,
            cli_listen,
            cli_cache,
            cli_substitute_urls,
            cli_policy,
        )
    }

    fn load_from_file(
        file: ConfigFile,
        cli_bootstrap: Option<String>,
        cli_external_addresses: Option<String>,
        cli_listen: Option<String>,
        cli_cache: Option<String>,
        cli_substitute_urls: Option<String>,
        cli_policy: Option<SubstitutePolicy>,
    ) -> Self {
        let cache_dir = cli_cache
            .as_ref()
            .or(file.cache_dir.as_ref())
            .map(PathBuf::from)
            .unwrap_or_else(dirs_cache_dir);

        let enable_default_bootstrap_peers = file.enable_default_bootstrap_peers.unwrap_or(true);
        let mut bootstrap_peers = Vec::new();
        if enable_default_bootstrap_peers {
            bootstrap_peers.extend(default_bootstrap_peers());
        }
        if let Some(peers) = file.bootstrap_peers.as_ref() {
            bootstrap_peers.extend(split_csv(peers));
        }
        if let Some(peers) = cli_bootstrap.as_ref() {
            bootstrap_peers.extend(split_csv(peers));
        }
        bootstrap_peers = dedup_strings(bootstrap_peers);

        Self {
            bootstrap_peers,
            enable_default_bootstrap_peers,
            peer_store_enabled: file.peer_store_enabled.unwrap_or(true),
            peer_store_max_entries: file.peer_store_max_entries.unwrap_or(100),
            external_addresses: cli_external_addresses
                .map(|s| split_csv(&s))
                .or_else(|| file.external_addresses.as_ref().map(|s| split_csv(s)))
                .unwrap_or_default(),
            listen_addr: cli_listen
                .or(file.listen_addr)
                .unwrap_or_else(|| "/ip4/0.0.0.0/udp/6881/quic-v1".into()),
            cache_dir: cache_dir.clone(),
            block_size: file.block_size.unwrap_or(262144),
            request_timeout_secs: file.request_timeout_secs.unwrap_or(30),
            stall_timeout_secs: file.stall_timeout_secs.unwrap_or(30),
            max_peers_per_download: file.max_peers_per_download.unwrap_or(8),
            max_in_flight_blocks_per_peer: file.max_in_flight_blocks_per_peer.unwrap_or(4),
            max_upload_rate_kbps: file.max_upload_rate_kbps,
            max_download_rate_kbps: file.max_download_rate_kbps,
            substitute_urls: cli_substitute_urls
                .map(|s| split_csv(&s))
                .or_else(|| file.substitute_urls.as_ref().map(|s| split_csv(s)))
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
            auto_seed_downloads: file
                .auto_seed_downloads
                .map(AutoSeedDownloadsConfig::into_mode)
                .unwrap_or_default(),
            local_narinfo_path: file.local_narinfo_path.map(PathBuf::from),
        }
    }
}

pub fn user_config_path() -> PathBuf {
    config_dir().join("config.toml")
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
    let path = user_config_path();
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

pub fn write_initial_config(
    path: &Path,
    options: InitConfigOptions,
) -> anyhow::Result<InitConfigResult> {
    if path.exists() {
        return Ok(InitConfigResult::AlreadyExists);
    }
    let parent = path.parent().ok_or_else(|| anyhow::anyhow!("config path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    std::fs::write(path, initial_config_template(options))?;
    Ok(InitConfigResult::Created)
}

fn initial_config_template(options: InitConfigOptions) -> String {
    let listen_addr =
        options.listen_addr.unwrap_or_else(|| "/ip4/0.0.0.0/udp/6881/quic-v1".to_string());
    let cache_dir = options.cache_dir.unwrap_or_else(|| dirs_cache_dir().display().to_string());
    let socket_path = options
        .socket_path
        .unwrap_or_else(|| PathBuf::from(&cache_dir).join("guix-p2p.sock").display().to_string());
    let substitute_urls =
        options.substitute_urls.unwrap_or_else(|| default_substitute_urls().join(","));
    let policy = options.substitute_policy.unwrap_or_default();
    let bootstrap_peers = options.bootstrap_peers.unwrap_or_default();
    let external_addresses = options.external_addresses.unwrap_or_default();

    format!(
        r#"# guix-p2p tester configuration
# Run `guix-p2p --doctor` after editing this file.
# To let remote peers dial this node:
# 1. Keep cache_dir stable so the PeerId stays stable.
# 2. Set external_addresses to the public DNS/IP multiaddr.
# 3. Start the daemon, then run `guix-p2p --share-info`.
# 4. Share the generated bootstrap_peers snippet with testers.

listen_addr = {listen_addr}
bootstrap_peers = {bootstrap_peers}
enable_default_bootstrap_peers = true
peer_store_enabled = true
peer_store_max_entries = 100

# Set this when peers outside your LAN should dial this node.
# Example: "/dns4/node.example.org/udp/6881/quic-v1"
# Run `guix-p2p --share-info` after setting this to see the full /p2p address.
external_addresses = {external_addresses}

cache_dir = {cache_dir}
socket_path = {socket_path}

substitute_policy = {policy}
substitute_urls = {substitute_urls}
min_providers = 3

dashboard_enabled = true
dashboard_bind = "127.0.0.1"
dashboard_port = 3030

# Cache successful downloads and serve them to peers: "off", "p2p", or "all".
auto_seed_downloads = "p2p"
seed_paths = []
"#,
        listen_addr = toml_string(&listen_addr),
        bootstrap_peers = toml_string(&bootstrap_peers),
        external_addresses = toml_string(&external_addresses),
        cache_dir = toml_string(&cache_dir),
        socket_path = toml_string(&socket_path),
        policy = toml_string(&policy.to_string()),
        substitute_urls = toml_string(&substitute_urls),
    )
}

fn toml_string(value: &str) -> String {
    toml_edit::Value::from(value).to_string()
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

fn split_csv(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|part| !part.is_empty()).map(str::to_string).collect()
}

pub fn dedup_strings(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

fn default_substitute_urls() -> Vec<String> {
    vec!["https://bordeaux.guix.gnu.org".into(), "https://ci.guix.gnu.org".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::load_from_file(
            ConfigFile::default(),
            None,
            None,
            None,
            Some("/tmp/guix-p2p-test".into()),
            None,
            None,
        );
        assert_eq!(config.block_size, 262144);
        assert!(config.bootstrap_peers.is_empty());
        assert!(config.enable_default_bootstrap_peers);
        assert!(config.peer_store_enabled);
        assert_eq!(config.peer_store_max_entries, 100);
        assert!(config.external_addresses.is_empty());
        assert_eq!(config.max_peers_per_download, 8);
        assert_eq!(config.max_in_flight_blocks_per_peer, 4);
        assert_eq!(config.max_upload_rate_kbps, None);
        assert_eq!(config.max_download_rate_kbps, None);
        assert_eq!(config.substitute_urls.len(), 2);
        assert_eq!(config.min_providers, 3);
        assert_eq!(config.stall_timeout_secs, 30);
        assert_eq!(config.dashboard_port, 3030);
        assert!(!config.dashboard_enabled);
        assert_eq!(config.substitute_policy, SubstitutePolicy::HttpFirst);
        assert_eq!(config.auto_seed_downloads, AutoSeedDownloads::P2p);
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

    #[test]
    fn test_auto_seed_downloads_from_str() {
        assert_eq!("off".parse(), Ok(AutoSeedDownloads::Off));
        assert_eq!("p2p".parse(), Ok(AutoSeedDownloads::P2p));
        assert_eq!("all".parse(), Ok(AutoSeedDownloads::All));
        assert!("invalid".parse::<AutoSeedDownloads>().is_err());
    }

    #[test]
    fn test_auto_seed_downloads_config_deserializes_modes_and_legacy_bools() {
        let file: ConfigFile = toml::from_str(r#"auto_seed_downloads = "all""#).unwrap();
        assert_eq!(file.auto_seed_downloads.unwrap().into_mode(), AutoSeedDownloads::All);

        let file: ConfigFile = toml::from_str("auto_seed_downloads = false").unwrap();
        assert_eq!(file.auto_seed_downloads.unwrap().into_mode(), AutoSeedDownloads::Off);

        let file: ConfigFile = toml::from_str("auto_seed_downloads = true").unwrap();
        assert_eq!(file.auto_seed_downloads.unwrap().into_mode(), AutoSeedDownloads::All);
    }

    #[test]
    fn test_rate_limit_config_deserializes() {
        let file: ConfigFile = toml::from_str(
            r#"
            max_upload_rate_kbps = 512
            max_download_rate_kbps = 1024
            "#,
        )
        .unwrap();

        assert_eq!(file.max_upload_rate_kbps, Some(512));
        assert_eq!(file.max_download_rate_kbps, Some(1024));
    }

    #[test]
    fn test_bootstrap_peers_merge_and_deduplicate() {
        let peers = dedup_strings(split_csv(
            "/ip4/127.0.0.1/tcp/1/p2p/a, /ip4/127.0.0.1/tcp/1/p2p/a,/ip4/127.0.0.1/tcp/2/p2p/b",
        ));

        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0], "/ip4/127.0.0.1/tcp/1/p2p/a");
        assert_eq!(peers[1], "/ip4/127.0.0.1/tcp/2/p2p/b");
    }

    #[test]
    fn init_config_creates_template_without_overwriting() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("guix-p2p/config.toml");

        let result = write_initial_config(
            &path,
            InitConfigOptions {
                bootstrap_peers: Some(
                    "/dns4/bootstrap.example.org/udp/6881/quic-v1/p2p/12D3KooWboot".to_string(),
                ),
                external_addresses: Some("/dns4/node.example.org/udp/6881/quic-v1".to_string()),
                substitute_policy: Some(SubstitutePolicy::HttpFirst),
                ..InitConfigOptions::default()
            },
        )
        .unwrap();

        assert_eq!(result, InitConfigResult::Created);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("bootstrap.example.org"));
        assert!(content.contains("node.example.org"));
        assert!(content.contains("substitute_policy = \"http-first\""));

        std::fs::write(&path, "sentinel = true\n").unwrap();
        let result = write_initial_config(&path, InitConfigOptions::default()).unwrap();

        assert_eq!(result, InitConfigResult::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "sentinel = true\n");
    }
}
