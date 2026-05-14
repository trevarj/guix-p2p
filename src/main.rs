use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use clap::Parser;
use guix_p2p::{
    channel::{SwarmCommand, SwarmNotification},
    config::{self, SubstitutePolicy},
    connection::{ConnectionConfig, ConnectionManager},
    daemon, dashboard, dht, identity, nar_store, narinfo,
    reputation::ReputationTracker,
};

#[derive(Parser)]
#[command(name = "guix-p2p", version)]
struct Cli {
    /// Run in query mode (driven by guix-daemon --query)
    #[arg(long, conflicts_with_all = ["substitute", "daemon"])]
    query: bool,

    /// Run in substitute mode (driven by guix-daemon --substitute)
    #[arg(long, conflicts_with_all = ["query", "daemon"])]
    substitute: bool,

    /// Run as a persistent background daemon
    #[arg(long, conflicts_with_all = ["query", "substitute"])]
    daemon: bool,

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

    /// JSON metadata file for offline narinfo lookups
    #[arg(long, global = true)]
    local_narinfo: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

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
        config.seed_paths = seed.split(',').map(str::to_string).collect();
    }
    if let Some(ref path) = cli.local_narinfo {
        config.local_narinfo_path = Some(path.clone());
    }
    if let Some(min_providers) = cli.min_providers {
        config.min_providers = min_providers;
    }

    tracing::info!("Starting guix-p2p");
    tracing::info!("Cache directory: {}", config.cache_dir.display());
    tracing::info!("Substitute policy: {}", config.substitute_policy);

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
    tracing::info!("Peer ID: {}", peer_id);

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

    dht::bootstrap(&mut swarm, &config.bootstrap_peers)?;

    // Initialize nar store and announce all seeded nars in the DHT
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

    let provider_cache = dht::create_provider_cache();
    let narinfo_cache = std::sync::Mutex::new(narinfo::NarinfoCache::new(60));
    let narinfo_cache = std::sync::Arc::new(narinfo_cache);
    load_local_narinfo_metadata(&config, &narinfo_cache);

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

    // Emit SeedAdded events for pre-seeded nars
    {
        let store = nar_store.lock().unwrap();
        for hash in store.seeded_hashes() {
            let info = store.seed_info(&hash);
            let nar_size = info.as_ref().map_or(0, |i| i.nar_size);
            let store_path = info.and_then(|i| i.store_path);
            let _ = event_tx.send(dashboard::DashboardEvent::SeedAdded {
                nar_hash: hash.clone(),
                store_path,
                nar_size,
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
            &http_client,
            &nar_store,
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
        )
        .await?
    } else {
        tracing::error!("No mode specified. Use --query, --substitute, or --daemon.");
        std::process::exit(1);
    }

    let _ = reputation.lock().unwrap().save(&rep_path);

    Ok(())
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
