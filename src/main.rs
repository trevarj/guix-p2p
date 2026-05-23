use std::{collections::HashMap, io::IsTerminal, sync::Arc};

use anyhow::Context;
use clap::{ArgGroup, Parser};
use guix_p2p::{
    bandwidth::{BandwidthConfig, BandwidthLimiter},
    channel::{SwarmCommand, SwarmNotification},
    config::{self, AutoSeedDownloads, SubstitutePolicy},
    connection::{ConnectionConfig, ConnectionManager},
    daemon, dashboard, dht, identity, nar_store, narinfo,
    reputation::ReputationTracker,
};

#[derive(Parser)]
#[command(name = "guix-p2p", version = guix_p2p::version::VERSION)]
#[command(group(ArgGroup::new("json_output_mode").args(["doctor", "share_info", "test_connectivity"])))]
struct Cli {
    /// Run in query mode (driven by guix-daemon --query)
    #[arg(long, conflicts_with_all = ["substitute", "daemon"])]
    query: bool,

    /// Run in substitute mode (driven by guix-daemon --substitute)
    #[arg(long, conflicts_with_all = ["query", "daemon"])]
    substitute: bool,

    /// Run as a persistent background daemon
    #[arg(long, conflicts_with_all = ["query", "substitute", "doctor", "init"])]
    daemon: bool,

    /// Run local readiness checks for tester rollout and connectivity setup
    #[arg(long, conflicts_with_all = ["query", "substitute", "daemon", "init", "share_info", "test_connectivity"])]
    doctor: bool,

    /// Emit machine-readable JSON for --doctor or --share-info
    #[arg(long, requires = "json_output_mode")]
    json: bool,

    /// Create a starter config file without overwriting an existing one
    #[arg(long, conflicts_with_all = ["query", "substitute", "daemon", "doctor", "share_info", "test_connectivity"])]
    init: bool,

    /// Print shareable bootstrap information for testers
    #[arg(long, conflicts_with_all = ["query", "substitute", "daemon", "doctor", "init", "test_connectivity"])]
    share_info: bool,

    /// Actively dial a peer multiaddr and report whether the connection succeeds
    #[arg(long, value_name = "MULTIADDR", conflicts_with_all = ["query", "substitute", "daemon", "doctor", "init", "share_info"])]
    test_connectivity: Option<String>,

    /// Comma-separated list of bootstrap peer multiaddrs
    #[arg(long, global = true)]
    bootstrap_peers: Option<String>,

    /// Comma-separated externally reachable listener addresses to advertise
    #[arg(long, global = true)]
    external_addresses: Option<String>,

    /// Address to listen on (multiaddr format)
    #[arg(long, global = true)]
    listen_addr: Option<String>,

    /// Directory for cache and identity storage
    #[arg(long, global = true)]
    cache_dir: Option<String>,

    /// Comma-separated substitute URLs for HTTP fallback
    #[arg(long, global = true)]
    substitute_urls: Option<String>,

    /// Substitute download policy: p2p-only, p2p-first, or http-first
    #[arg(long, global = true)]
    policy: Option<SubstitutePolicy>,

    /// Minimum P2P providers required before claiming or downloading a nar
    #[arg(long, global = true)]
    min_providers: Option<usize>,

    /// Enable web dashboard in daemon mode
    #[arg(long)]
    dashboard: bool,

    /// Port for web dashboard
    #[arg(long, default_value = "3030")]
    dashboard_port: u16,

    /// Bind address for web dashboard
    #[arg(long, default_value = "127.0.0.1")]
    dashboard_bind: String,

    /// SOCKS5 proxy address for Tor (e.g. 127.0.0.1:9050)
    #[arg(long, global = true)]
    tor_socks: Option<String>,

    /// Route all traffic through Tor only (no direct connections)
    #[arg(long, global = true)]
    tor_only: bool,

    /// Unix socket path for daemon to bind and relay to connect
    #[arg(long, global = true)]
    socket: Option<String>,

    /// Comma-separated store paths to seed via guix archive --export
    #[arg(long, global = true)]
    seed: Option<String>,

    /// Do not cache successful substitute downloads for later P2P seeding
    #[arg(long, global = true, conflicts_with = "auto_seed_downloads")]
    no_auto_seed_downloads: bool,

    /// Which successful downloads to auto-seed: off, p2p, or all
    #[arg(long, global = true)]
    auto_seed_downloads: Option<AutoSeedDownloads>,

    /// JSON metadata file for offline narinfo lookups
    #[arg(long, global = true)]
    local_narinfo: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let relay_mode = (cli.query || cli.substitute) && cli.socket.is_some();
    let default_log_filter = if relay_mode
        || cli.doctor
        || cli.init
        || cli.share_info
        || cli.test_connectivity.is_some()
    {
        "warn"
    } else {
        "info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_log_filter.into()),
        )
        .init();

    if cli.init {
        let path = config::user_config_path();
        let result = config::write_initial_config(
            &path,
            config::InitConfigOptions {
                bootstrap_peers: cli.bootstrap_peers.clone(),
                external_addresses: cli.external_addresses.clone(),
                listen_addr: cli.listen_addr.clone(),
                cache_dir: cli.cache_dir.clone(),
                substitute_urls: cli.substitute_urls.clone(),
                substitute_policy: cli.policy,
                socket_path: cli.socket.clone(),
            },
        )?;
        match result {
            config::InitConfigResult::Created => {
                println!("created {}", path.display());
                println!(
                    "next: edit bootstrap_peers/external_addresses, then run guix-p2p --doctor"
                );
            },
            config::InitConfigResult::AlreadyExists => {
                println!("exists {}", path.display());
                println!("not overwriting existing config");
            },
        }
        return Ok(());
    }

    let mut config = config::Config::load(
        cli.bootstrap_peers,
        cli.external_addresses,
        cli.listen_addr,
        cli.cache_dir,
        cli.substitute_urls,
        cli.policy,
    );

    let dashboard_enabled = cli.dashboard;
    if dashboard_enabled {
        config.dashboard_enabled = true;
        config.dashboard_port = cli.dashboard_port;
        config.dashboard_bind = cli.dashboard_bind;
    }

    if let Some(ref proxy) = cli.tor_socks {
        config.tor_socks = Some(proxy.clone());
        config.tor_only = cli.tor_only;
    }

    if let Some(ref sock) = cli.socket {
        config.socket_path = sock.clone();
    }

    if let Some(ref seed) = cli.seed {
        config.seed_paths = seed
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect();
    }
    if cli.no_auto_seed_downloads {
        config.auto_seed_downloads = AutoSeedDownloads::Off;
    }
    if let Some(mode) = cli.auto_seed_downloads {
        config.auto_seed_downloads = mode;
    }
    if let Some(ref path) = cli.local_narinfo {
        config.local_narinfo_path = Some(path.clone());
    }
    if let Some(min_providers) = cli.min_providers {
        config.min_providers = min_providers;
    }

    match (cli.query, cli.substitute, &cli.socket) {
        (true, false, Some(sock)) => {
            guix_p2p::relay::forward(sock, guix_p2p::relay::RelayMode::Query).await?;
            return Ok(());
        },
        (false, true, Some(sock)) => {
            guix_p2p::relay::forward(sock, guix_p2p::relay::RelayMode::Substitute).await?;
            return Ok(());
        },
        _ => {},
    }

    let keypair = identity::load_or_generate_keypair(&config.cache_dir)
        .context("failed to load or generate identity")?;

    let peer_id = identity::peer_id_from_keypair(&keypair);
    tracing::info!(
        version = %guix_p2p::version::VERSION,
        peer_id = %peer_id,
        cache_dir = %config.cache_dir.display(),
        policy = %config.substitute_policy,
        listen_addr = %config.listen_addr,
        socket = %config.socket_path,
        dashboard = config.dashboard_enabled,
        dashboard_bind = %config.dashboard_bind,
        dashboard_port = config.dashboard_port,
        "Starting guix-p2p"
    );

    if cli.doctor {
        let peer_id = peer_id.to_string();
        let report = guix_p2p::diagnostics::diagnostic_report(&config, &peer_id);
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print!(
                "{}",
                guix_p2p::diagnostics::format_diagnostics_with_color(
                    &report.checks,
                    stdout_color_enabled()
                )
            );
        }
        if report.has_errors {
            std::process::exit(2);
        }
        return Ok(());
    }

    if cli.share_info {
        let peer_id = peer_id.to_string();
        let bundle = guix_p2p::diagnostics::bootstrap_bundle(&config, &peer_id);
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&bundle)?);
        } else {
            print!("{}", guix_p2p::diagnostics::format_bootstrap_bundle(&bundle));
        }
        return Ok(());
    }

    if let Some(peer_addr) = cli.test_connectivity.as_deref() {
        let report = guix_p2p::connectivity::test_peer_connectivity(
            &keypair,
            peer_addr,
            std::time::Duration::from_secs(config.request_timeout_secs),
        )
        .await?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print!("{}", guix_p2p::connectivity::format_connectivity_test(&report));
        }
        if !report.success {
            std::process::exit(2);
        }
        return Ok(());
    }

    let mut swarm = guix_p2p::runtime::build_swarm(&keypair)?;

    let listen_addr: libp2p::Multiaddr =
        config.listen_addr.parse().context("failed to parse listen address")?;
    swarm.listen_on(listen_addr).context("failed to listen")?;

    for addr in &config.external_addresses {
        let addr: libp2p::Multiaddr =
            addr.parse().with_context(|| format!("failed to parse external address {addr}"))?;
        tracing::info!("Advertising external address {}", addr);
        swarm.add_external_address(addr);
    }

    let peer_store = if config.peer_store_enabled {
        let store =
            guix_p2p::peer_store::PeerStore::load(&config.cache_dir, config.peer_store_max_entries);
        Some(Arc::new(std::sync::Mutex::new(store)))
    } else {
        None
    };
    let mut bootstrap_peers = config.bootstrap_peers.clone();
    if let Some(store) = &peer_store {
        bootstrap_peers.extend(store.lock().unwrap().bootstrap_peers());
        bootstrap_peers = config::dedup_strings(bootstrap_peers);
    }
    dht::bootstrap(&mut swarm, &bootstrap_peers)?;

    let provider_cache = dht::create_provider_cache();
    let narinfo_cache = std::sync::Mutex::new(narinfo::NarinfoCache::new(60));
    let narinfo_cache = std::sync::Arc::new(narinfo_cache);
    load_local_narinfo_metadata(&config, &narinfo_cache);

    // Initialize nar store and announce all seeded nars in the DHT.
    let nar_store = Arc::new(std::sync::Mutex::new(nar_store::NarStore::new(
        &config.cache_dir,
        config.block_size,
    )));
    {
        let mut store = nar_store.lock().unwrap();
        for path in &config.seed_paths {
            match store.seed_store_path(path) {
                Ok(hash) => tracing::info!("seeded {} -> {}..", path, &hash[..16]),
                Err(e) => tracing::warn!("failed to seed {}: {}", path, e),
            }
        }
        let narinfos = narinfo_cache.lock().unwrap().active_entries();
        let annotated = store.annotate_from_narinfos(&narinfos);
        if annotated > 0 {
            tracing::info!("attached narinfo metadata to {} cached nar(s)", annotated);
        }
        for hash in store.seeded_hashes() {
            if let Ok(bytes) = hex::decode(&hash) {
                let key = libp2p::kad::RecordKey::new(&bytes);
                if let Err(e) = swarm.behaviour_mut().kad.start_providing(key) {
                    tracing::warn!("failed to announce nar {}: {}", &hash[..16], e);
                } else {
                    tracing::info!("announced nar {}.. in DHT", &hash[..16]);
                }
            }
        }
    }

    let http_client = guix_p2p::http_client::create_http_client(&config)
        .context("failed to create HTTP client")?;

    let conn_config = ConnectionConfig {
        connect_timeout: std::time::Duration::from_secs(config.request_timeout_secs),
        max_retries: config.connection_retries,
        backoff_base: std::time::Duration::from_secs(1),
        health_check_interval: std::time::Duration::from_secs(config.health_check_interval_secs),
        max_peers: config.max_total_peers,
    };
    let rep_path = config.cache_dir.join("reputation.json");
    let reputation = Arc::new(std::sync::Mutex::new(
        ReputationTracker::load(&rep_path, config.reputation_ban_threshold)
            .unwrap_or_else(|_| ReputationTracker::new(config.reputation_ban_threshold)),
    ));
    let conn_mgr = Arc::new(std::sync::Mutex::new(ConnectionManager::new(conn_config)));

    let (event_tx, _) = tokio::sync::broadcast::channel::<dashboard::DashboardEvent>(256);
    let build_registry: dashboard::BuildRegistry = Arc::new(std::sync::Mutex::new(HashMap::new()));
    let bandwidth_limiter = Arc::new(BandwidthLimiter::new(BandwidthConfig {
        upload_limit_bytes_per_sec: config
            .max_upload_rate_kbps
            .map(|kbps| kbps.saturating_mul(1024)),
        download_limit_bytes_per_sec: config
            .max_download_rate_kbps
            .map(|kbps| kbps.saturating_mul(1024)),
    }));

    // Emit SeedAdded events for pre-seeded nars
    {
        let store = nar_store.lock().unwrap();
        for hash in store.seeded_hashes() {
            let info = store.seed_info(&hash);
            let nar_size = info.as_ref().map_or(0, |i| i.nar_size);
            let source = info.as_ref().map_or("cache", |i| i.source.as_str()).to_string();
            let store_path = info.and_then(|i| i.store_path);
            let _ = event_tx.send(dashboard::DashboardEvent::SeedAdded {
                nar_hash: hash.clone(),
                store_path,
                nar_size,
                source,
            });
        }
    }

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<SwarmCommand>();
    let (notify_tx, _) = tokio::sync::broadcast::channel::<SwarmNotification>(4096);
    let (query_tx, query_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let cache_for_swarm = provider_cache.clone();
    let notify_for_kad = notify_tx.clone();
    let cmd_rx_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(cmd_rx);
    let rep_for_swarm = reputation.clone();
    let conn_for_swarm = conn_mgr.clone();
    let evt_for_swarm = event_tx.clone();
    let nar_store_for_swarm = nar_store.clone();
    let limiter_for_swarm = bandwidth_limiter.clone();
    let peer_store_for_swarm = peer_store.clone();

    tokio::spawn(async move {
        guix_p2p::runtime::run_swarm_task(
            swarm,
            cache_for_swarm,
            notify_for_kad,
            cmd_rx_stream,
            query_rx,
            rep_for_swarm,
            conn_for_swarm,
            evt_for_swarm,
            nar_store_for_swarm,
            limiter_for_swarm,
            peer_store_for_swarm,
        )
        .await;
    });

    tracing::info!("Swarm task spawned, entering daemon event loop");

    let notify_rx = notify_tx.subscribe();

    if cli.query {
        daemon::run_query_mode(
            &provider_cache,
            &query_tx,
            &cmd_tx,
            notify_rx,
            &narinfo_cache,
            &config,
            &http_client,
        )
        .await?
    } else if cli.substitute {
        daemon::run_substitute_mode(
            &provider_cache,
            &cmd_tx,
            notify_rx,
            &narinfo_cache,
            &config,
            &query_tx,
            &reputation,
            &conn_mgr,
            &http_client,
            &nar_store,
            &bandwidth_limiter,
        )
        .await?
    } else if cli.daemon {
        daemon::run_daemon_mode(
            &provider_cache,
            &cmd_tx,
            &query_tx,
            &notify_tx,
            &narinfo_cache,
            &config,
            &reputation,
            &conn_mgr,
            &build_registry,
            &event_tx,
            &http_client,
            &nar_store,
            &peer_id.to_string(),
            &bandwidth_limiter,
        )
        .await?
    } else {
        tracing::error!(
            "No mode specified. Use --query, --substitute, --daemon, --doctor, --share-info, \
             --test-connectivity, or --init."
        );
        std::process::exit(1);
    }

    let _ = reputation.lock().unwrap().save(&rep_path);

    Ok(())
}

fn stdout_color_enabled() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

#[derive(Debug, serde::Deserialize)]
struct LocalNarinfoFile {
    narinfos: Vec<LocalNarinfoEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct LocalNarinfoEntry {
    store_path: String,
    nar_hash: String,
    nar_size: u64,
    #[serde(default)]
    references: Vec<String>,
    deriver: Option<String>,
    #[serde(default)]
    download_size: u64,
}

fn load_local_narinfo_metadata(
    config: &guix_p2p::config::Config,
    cache: &std::sync::Arc<std::sync::Mutex<narinfo::NarinfoCache>>,
) {
    let Some(path) = &config.local_narinfo_path else {
        return;
    };

    match load_local_narinfos(path) {
        Ok(narinfos) => {
            let mut guard = cache.lock().unwrap();
            for info in narinfos {
                match guix_p2p::store_path::hash_part(&info.store_path) {
                    Ok(hash_part) => guard.put(hash_part, info),
                    Err(e) => tracing::warn!("skipping local narinfo: {}", e),
                }
            }
            tracing::info!("loaded local narinfo metadata from {}", path.display());
        },
        Err(e) => tracing::warn!("failed to load local narinfo metadata {}: {}", path.display(), e),
    }
}

fn load_local_narinfos(path: &std::path::Path) -> anyhow::Result<Vec<narinfo::Narinfo>> {
    let file: LocalNarinfoFile = serde_json::from_str(
        &std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", path.display()))?;

    Ok(file
        .narinfos
        .into_iter()
        .map(|entry| narinfo::Narinfo {
            store_path: entry.store_path,
            nar_hash: entry.nar_hash,
            nar_size: entry.nar_size,
            references: entry.references,
            deriver: entry.deriver,
            urls: vec![narinfo::NarUrl {
                url: "p2p://local-metadata".to_string(),
                compression: "none".to_string(),
                file_size: entry.download_size,
            }],
            signed_portion: "local narinfo metadata".to_string(),
            signature: None,
        })
        .collect())
}
