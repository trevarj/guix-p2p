use std::{
    collections::{HashMap, HashSet},
    env, fs,
    net::{IpAddr, SocketAddr},
    path::{Path as FsPath, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{delete, get},
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use toml_edit::{Array, DocumentMut, Item, Value};

use crate::{
    channel::SwarmCommand, connection::ConnectionManager, dht::ProviderCache, nar_store::NarStore,
    reputation::ReputationTracker,
};

pub type EventBus = tokio::sync::broadcast::Sender<DashboardEvent>;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum DashboardEvent {
    PeerConnected {
        peer_id: String,
        addresses: Vec<String>,
    },
    PeerDisconnected {
        peer_id: String,
    },
    ProvidersFound {
        nar_hash: String,
        provider_count: usize,
    },
    BuildDiscovered {
        nar_hash: String,
        store_path: Option<String>,
        nar_size: Option<u64>,
    },
    DownloadStarted {
        nar_hash: String,
        store_path: String,
        nar_size: u64,
    },
    BlockReceived {
        nar_hash: String,
        peer_id: String,
        blocks: usize,
    },
    DownloadSucceeded {
        nar_hash: String,
        store_path: String,
        size: u64,
        elapsed_ms: u64,
    },
    DownloadFailed {
        nar_hash: String,
        store_path: String,
        reason: String,
    },
    SeedAdded {
        nar_hash: String,
        store_path: Option<String>,
        nar_size: u64,
    },
    SeedRemoved {
        nar_hash: String,
        store_path: Option<String>,
    },
    BlockServed {
        nar_hash: String,
        peer_id: String,
        indices: Vec<u32>,
    },
    CatalogEntry {
        hash_part: String,
        store_path: Option<String>,
        nar_size: Option<u64>,
        nar_hash: Option<String>,
        p2p_available: bool,
    },
}

pub type BuildRegistry = Arc<Mutex<HashMap<String, ObservedBuild>>>;

#[derive(Debug, Clone, Serialize)]
pub struct ObservedBuild {
    pub nar_hash: String,
    pub store_path: Option<String>,
    pub nar_size: Option<u64>,
    pub references: Vec<String>,
    pub deriver: Option<String>,
    pub narinfo_raw: Option<String>,
    pub providers: Vec<String>,
    pub downloaded_at: Option<u64>,
    pub download_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct ApiPeer {
    peer_id: String,
    score: f64,
    completed: u32,
    failed: u32,
    bytes_served: u64,
    connected: bool,
    addresses: Vec<String>,
    country: Option<String>,
    ip: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ApiStatus {
    peer_id: String,
    uptime_secs: u64,
    connected_peers: usize,
    dht_entries: usize,
    build_count: usize,
    seed_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct ApiBuild {
    lookup_key: String,
    nar_hash: String,
    store_path: Option<String>,
    nar_size: Option<u64>,
    provider_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct ApiSeededNar {
    nar_hash: String,
    nar_size: u64,
    block_count: u32,
    block_size: u32,
    store_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ApiCatalogEntry {
    hash_part: String,
    store_path: Option<String>,
    nar_size: Option<u64>,
    nar_hash: Option<String>,
    p2p_available: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ApiPackage {
    source: String,
    name: String,
    version: String,
    output: String,
    store_path: String,
    seeded: bool,
}

#[derive(Debug, Deserialize)]
struct ApiSeedRequest {
    store_path: String,
}

#[derive(Debug, Serialize)]
struct ApiSeedMutation {
    nar_hash: String,
    store_path: String,
    nar_size: u64,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
}

#[derive(Debug, Clone)]
pub struct CatalogItem {
    hash_part: String,
    store_path: Option<String>,
    nar_size: Option<u64>,
    nar_hash: Option<String>,
    p2p_available: bool,
}

#[derive(Clone)]
pub struct DashboardState {
    pub provider_cache: ProviderCache,
    pub reputation: Arc<Mutex<ReputationTracker>>,
    pub conn_mgr: Arc<Mutex<ConnectionManager>>,
    pub build_registry: BuildRegistry,
    pub started: Instant,
    pub peer_id: String,
    pub event_bus: EventBus,
    pub nar_store: Arc<Mutex<NarStore>>,
    pub catalog: Arc<Mutex<HashMap<String, CatalogItem>>>,
    pub cmd_tx: UnboundedSender<SwarmCommand>,
    pub seed_mutation_allowed: bool,
}

pub async fn serve(state: DashboardState, port: u16, bind: &str) {
    let addr: SocketAddr =
        format!("{}:{}", bind, port).parse().expect("invalid dashboard bind address");

    let catalog_state = state.clone();
    tokio::spawn(async move {
        maintain_catalog(catalog_state).await;
    });

    let app = Router::new()
        .route("/", get(index_html))
        .route("/api/status", get(api_status))
        .route("/api/peers", get(api_peers))
        .route("/api/builds", get(api_builds))
        .route("/api/build/{hash}", get(api_build_detail))
        .route("/api/catalog", get(api_catalog))
        .route("/api/seeds", get(api_seeds).post(api_seed))
        .route("/api/seeds/{hash}", delete(api_seed_delete))
        .route("/api/packages", get(api_packages))
        .route("/ws", get(ws_handler))
        .with_state(state);

    tracing::info!("Dashboard listening on http://{}", addr);

    let listener =
        tokio::net::TcpListener::bind(addr).await.expect("failed to bind dashboard port");
    axum::serve(listener, app).await.expect("dashboard server error");
}

async fn index_html() -> Html<&'static str> {
    Html(include_str!("dashboard.html"))
}

async fn api_status(State(state): State<DashboardState>) -> Json<ApiStatus> {
    let now = state.started.elapsed().as_secs();
    let dht = state.provider_cache.lock().await.len();
    let builds = state.build_registry.lock().unwrap().len();
    let seed_count = state.nar_store.lock().unwrap().len();
    let connected = state.conn_mgr.lock().unwrap().connected_count();

    let status = ApiStatus {
        peer_id: state.peer_id.clone(),
        uptime_secs: now,
        connected_peers: connected,
        dht_entries: dht,
        build_count: builds,
        seed_count,
    };

    Json(status)
}

async fn api_peers(State(state): State<DashboardState>) -> Json<Vec<ApiPeer>> {
    let rep = state.reputation.lock().unwrap();
    let peers: Vec<ApiPeer> = rep
        .peer_entries()
        .into_iter()
        .map(|(peer, peer_score)| {
            let addrs = vec![];
            let (ip, country) = extract_addr_info(&addrs);
            ApiPeer {
                peer_id: peer.to_string(),
                score: peer_score.score(Instant::now()),
                completed: peer_score.completed,
                failed: peer_score.failed,
                bytes_served: peer_score.bytes_served,
                connected: false,
                addresses: addrs.clone(),
                country,
                ip,
            }
        })
        .collect();
    drop(rep);
    Json(peers)
}

async fn api_builds(State(state): State<DashboardState>) -> Json<Vec<ApiBuild>> {
    let reg = state.build_registry.lock().unwrap();
    let mut builds: Vec<ApiBuild> = reg
        .iter()
        .map(|(lookup_key, b)| ApiBuild {
            lookup_key: lookup_key.clone(),
            nar_hash: b.nar_hash.clone(),
            store_path: b.store_path.clone(),
            nar_size: b.nar_size,
            provider_count: b.providers.len(),
        })
        .collect();
    builds.sort_by(|a, b| {
        b.provider_count
            .cmp(&a.provider_count)
            .then_with(|| a.store_path.cmp(&b.store_path))
            .then_with(|| a.nar_hash.cmp(&b.nar_hash))
    });
    Json(builds)
}

async fn api_catalog(State(state): State<DashboardState>) -> Json<Vec<ApiCatalogEntry>> {
    let cat = state.catalog.lock().unwrap();
    let mut entries: Vec<ApiCatalogEntry> = cat
        .values()
        .map(|c| ApiCatalogEntry {
            hash_part: c.hash_part.clone(),
            store_path: c.store_path.clone(),
            nar_size: c.nar_size,
            nar_hash: c.nar_hash.clone(),
            p2p_available: c.p2p_available,
        })
        .collect();
    entries.sort_by(|a, b| {
        b.p2p_available
            .cmp(&a.p2p_available)
            .then_with(|| a.store_path.cmp(&b.store_path))
            .then_with(|| a.hash_part.cmp(&b.hash_part))
    });
    Json(entries)
}

async fn api_seeds(State(state): State<DashboardState>) -> Json<Vec<ApiSeededNar>> {
    let store = state.nar_store.lock().unwrap();
    let mut seeds: Vec<ApiSeededNar> = store
        .seeded_hashes()
        .into_iter()
        .filter_map(|hash| {
            let info = store.seed_info(&hash)?;
            Some(ApiSeededNar {
                nar_hash: hash,
                nar_size: info.nar_size,
                block_count: info.block_count,
                block_size: info.block_size,
                store_path: info.store_path,
            })
        })
        .collect();
    seeds.sort_by(|a, b| b.nar_size.cmp(&a.nar_size).then_with(|| a.nar_hash.cmp(&b.nar_hash)));
    Json(seeds)
}

async fn api_packages(State(state): State<DashboardState>) -> Json<Vec<ApiPackage>> {
    let seeded_store_paths: HashSet<String> = {
        let store = state.nar_store.lock().unwrap();
        store
            .seeded_hashes()
            .into_iter()
            .filter_map(|hash| store.seed_info(&hash).and_then(|info| info.store_path))
            .collect()
    };

    let mut packages = Vec::new();
    for (source, profile) in package_profiles() {
        packages.extend(installed_packages_from_profile(&source, &profile, &seeded_store_paths));
    }
    packages.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.version.cmp(&b.version))
            .then_with(|| a.output.cmp(&b.output))
            .then_with(|| a.store_path.cmp(&b.store_path))
    });

    Json(packages)
}

async fn api_seed(
    State(state): State<DashboardState>,
    Json(request): Json<ApiSeedRequest>,
) -> Result<Json<ApiSeedMutation>, (StatusCode, Json<ApiError>)> {
    if !state.seed_mutation_allowed {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "dashboard seed mutation requires a loopback bind address",
        ));
    }

    let store_path = request.store_path.trim().to_string();
    if !store_path.starts_with("/gnu/store/") {
        return Err(api_error(StatusCode::BAD_REQUEST, "store_path must start with /gnu/store/"));
    }
    if !FsPath::new(&store_path).exists() {
        return Err(api_error(StatusCode::NOT_FOUND, "store_path does not exist"));
    }

    let nar_store = state.nar_store.clone();
    let seed_path = store_path.clone();
    let nar_hash = tokio::task::spawn_blocking(move || {
        let mut store = nar_store.lock().unwrap();
        store.seed_store_path(&seed_path)
    })
    .await
    .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("seed task failed: {e}")))?
    .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("seed failed: {e}")))?;

    let info = state
        .nar_store
        .lock()
        .unwrap()
        .seed_info(&nar_hash)
        .ok_or_else(|| api_error(StatusCode::INTERNAL_SERVER_ERROR, "seed metadata missing"))?;

    state.cmd_tx.send(SwarmCommand::StartProviding { hash: nar_hash.clone() }).map_err(|e| {
        api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("failed to announce seed: {e}"))
    })?;

    persist_seed_path(&store_path).map_err(|e| {
        api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("persist failed: {e}"))
    })?;

    let _ = state.event_bus.send(DashboardEvent::SeedAdded {
        nar_hash: nar_hash.clone(),
        store_path: Some(store_path.clone()),
        nar_size: info.nar_size,
    });

    Ok(Json(ApiSeedMutation { nar_hash, store_path, nar_size: info.nar_size }))
}

async fn api_seed_delete(
    State(state): State<DashboardState>,
    Path(hash): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    if !state.seed_mutation_allowed {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "dashboard seed mutation requires a loopback bind address",
        ));
    }

    let info = state
        .nar_store
        .lock()
        .unwrap()
        .remove_seed(&hash)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "seed not found"))?;

    if let Some(store_path) = &info.store_path {
        remove_seed_path(store_path).map_err(|e| {
            api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("persist failed: {e}"))
        })?;
    }

    let _ = state
        .event_bus
        .send(DashboardEvent::SeedRemoved { nar_hash: hash, store_path: info.store_path });

    Ok(StatusCode::NO_CONTENT)
}

async fn api_build_detail(
    State(state): State<DashboardState>,
    Path(hash): Path<String>,
) -> Result<Json<ObservedBuild>, StatusCode> {
    let reg = state.build_registry.lock().unwrap();
    reg.get(&hash)
        .cloned()
        .or_else(|| reg.values().find(|build| build.nar_hash == hash).cloned())
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

fn api_error(status: StatusCode, message: &str) -> (StatusCode, Json<ApiError>) {
    (status, Json(ApiError { error: message.to_string() }))
}

pub fn seed_mutation_allowed_for_bind(bind: &str) -> bool {
    bind.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn package_profiles() -> Vec<(String, PathBuf)> {
    package_profiles_for_home(env::var_os("HOME").map(PathBuf::from))
}

fn package_profiles_for_home(home: Option<PathBuf>) -> Vec<(String, PathBuf)> {
    let mut profiles = vec![
        ("system".to_string(), PathBuf::from("/run/current-system/profile")),
        ("kernel".to_string(), PathBuf::from("/run/current-system/kernel")),
    ];
    if let Some(home) = home {
        profiles.push(("home".to_string(), home.join(".guix-home/profile")));
    }
    profiles
}

fn user_config_path() -> PathBuf {
    if let Ok(dir) = env::var("XDG_CONFIG_HOME") {
        PathBuf::from(dir).join("guix-p2p/config.toml")
    } else {
        let home = env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".config/guix-p2p/config.toml")
    }
}

fn persist_seed_path(store_path: &str) -> anyhow::Result<()> {
    persist_seed_path_to_config(store_path, &user_config_path())
}

fn remove_seed_path(store_path: &str) -> anyhow::Result<()> {
    remove_seed_path_from_config(store_path, &user_config_path())
}

fn persist_seed_path_to_config(store_path: &str, config_path: &FsPath) -> anyhow::Result<()> {
    let parent =
        config_path.parent().ok_or_else(|| anyhow::anyhow!("config path has no parent"))?;
    fs::create_dir_all(parent)?;

    let content = fs::read_to_string(config_path).unwrap_or_default();
    let mut doc = content.parse::<DocumentMut>().unwrap_or_else(|_| DocumentMut::new());

    let mut values = match doc.get("seed_paths").and_then(Item::as_array) {
        Some(existing) => {
            existing.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>()
        },
        None => Vec::new(),
    };
    if !values.iter().any(|value| value == store_path) {
        values.push(store_path.to_string());
    }

    let mut array = Array::default();
    for value in values {
        array.push(value);
    }
    doc["seed_paths"] = Item::Value(Value::Array(array));
    fs::write(config_path, doc.to_string())?;
    Ok(())
}

fn remove_seed_path_from_config(store_path: &str, config_path: &FsPath) -> anyhow::Result<()> {
    let content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(_) => return Ok(()),
    };
    let mut doc = content.parse::<DocumentMut>().unwrap_or_else(|_| DocumentMut::new());
    let values = match doc.get("seed_paths").and_then(Item::as_array) {
        Some(existing) => existing
            .iter()
            .filter_map(Value::as_str)
            .filter(|value| *value != store_path)
            .map(str::to_string)
            .collect::<Vec<_>>(),
        None => return Ok(()),
    };

    let mut array = Array::default();
    for value in values {
        array.push(value);
    }
    doc["seed_paths"] = Item::Value(Value::Array(array));
    fs::write(config_path, doc.to_string())?;
    Ok(())
}

fn installed_packages_from_profile(
    source: &str,
    profile: &FsPath,
    seeded_store_paths: &HashSet<String>,
) -> Vec<ApiPackage> {
    if !profile.exists() {
        return Vec::new();
    }

    let output = Command::new("guix")
        .arg("package")
        .arg("--list-installed")
        .arg(format!("--profile={}", profile.display()))
        .output();

    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            tracing::warn!(
                "failed to list Guix packages for {} profile {}: status {}",
                source,
                profile.display(),
                output.status,
            );
            return Vec::new();
        },
        Err(e) => {
            tracing::warn!(
                "failed to run guix package for {} profile {}: {}",
                source,
                profile.display(),
                e,
            );
            return Vec::new();
        },
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| parse_installed_package(source, line, seeded_store_paths))
        .collect()
}

fn parse_installed_package(
    source: &str,
    line: &str,
    seeded_store_paths: &HashSet<String>,
) -> Option<ApiPackage> {
    let mut fields = line.split('\t');
    let name = fields.next()?.trim();
    let version = fields.next()?.trim();
    let output = fields.next()?.trim();
    let store_path = fields.next()?.trim();

    if name.is_empty() || version.is_empty() || output.is_empty() || store_path.is_empty() {
        return None;
    }

    Some(ApiPackage {
        source: source.to_string(),
        name: name.to_string(),
        version: version.to_string(),
        output: output.to_string(),
        store_path: store_path.to_string(),
        seeded: seeded_store_paths.contains(store_path),
    })
}

async fn ws_handler(
    State(state): State<DashboardState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: DashboardState) {
    let mut rx = state.event_bus.subscribe();

    loop {
        let event = match rx.recv().await {
            Ok(e) => e,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::debug!("WebSocket lagged by {} events", n);
                continue;
            },
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };

        let json = match serde_json::to_string(&event) {
            Ok(j) => j,
            Err(_) => continue,
        };

        if socket.send(Message::Text(json)).await.is_err() {
            break;
        }
    }
}

async fn maintain_catalog(state: DashboardState) {
    let mut rx = state.event_bus.subscribe();

    loop {
        let event = match rx.recv().await {
            Ok(e) => e,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::debug!("Dashboard catalog listener lagged by {} events", n);
                continue;
            },
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };

        if let DashboardEvent::CatalogEntry {
            hash_part,
            store_path,
            nar_size,
            nar_hash,
            p2p_available,
        } = event
        {
            upsert_catalog_item(
                &state.catalog,
                hash_part,
                store_path,
                nar_size,
                nar_hash,
                p2p_available,
            );
        }
    }
}

fn upsert_catalog_item(
    catalog: &Arc<Mutex<HashMap<String, CatalogItem>>>,
    hash_part: String,
    store_path: Option<String>,
    nar_size: Option<u64>,
    nar_hash: Option<String>,
    p2p_available: bool,
) {
    let mut cat = catalog.lock().unwrap();
    cat.entry(hash_part.clone())
        .and_modify(|entry: &mut CatalogItem| {
            if store_path.is_some() {
                entry.store_path = store_path.clone();
            }
            if nar_size.is_some() {
                entry.nar_size = nar_size;
            }
            if nar_hash.is_some() {
                entry.nar_hash = nar_hash.clone();
            }
            entry.p2p_available |= p2p_available;
        })
        .or_insert_with(|| CatalogItem {
            hash_part,
            store_path,
            nar_size,
            nar_hash,
            p2p_available,
        });
}

fn extract_addr_info(addrs: &[String]) -> (Option<String>, Option<String>) {
    for addr in addrs {
        if let Some(ip) = extract_ip_from_multiaddr(addr) {
            let country = ip_to_country(&ip).map(|s| s.to_string());
            return (Some(ip), country);
        }
    }
    (None, None)
}

fn extract_ip_from_multiaddr(addr: &str) -> Option<String> {
    let ip4_marker = "/ip4/";
    if let Some(pos) = addr.find(ip4_marker) {
        let rest = &addr[pos + ip4_marker.len()..];
        return Some(rest.split('/').next()?.to_string());
    }
    let ip6_marker = "/ip6/";
    if let Some(pos) = addr.find(ip6_marker) {
        let rest = &addr[pos + ip6_marker.len()..];
        return Some(rest.split('/').next()?.to_string());
    }
    None
}

fn ip_to_country(ip: &str) -> Option<&'static str> {
    let first_octet = ip.split('.').next()?.parse::<u8>().ok()?;
    match first_octet {
        5 => Some("DE"),
        14 => Some("JP"),
        27 => Some("JP"),
        31 => Some("NL"),
        36 | 68 => Some("CA"),
        41 => Some("KE"),
        46 => Some("SE"),
        49 => Some("JP"),
        51 => Some("NO"),
        58 => Some("JP"),
        60 => Some("JP"),
        62 => Some("IT"),
        77..=83 => Some("RU"),
        84 => Some("ES"),
        85 | 86 => Some("CH"),
        87 => Some("DK"),
        88 => Some("PL"),
        89..=95 => Some("RU"),
        101 => Some("JP"),
        102 => Some("ZA"),
        103 => Some("JP"),
        105 => Some("FR"),
        106 => Some("JP"),
        109 => Some("IL"),
        110..=126 => Some("JP"),
        151 | 181 | 189 | 190 | 200 | 201 => Some("BR"),
        152 => Some("MX"),
        154 => Some("TR"),
        176 | 178 | 212 | 213 => Some("RU"),
        177 => Some("AR"),
        179 => Some("PE"),
        184 => Some("CL"),
        185 => Some("CZ"),
        187 => Some("PT"),
        188 => Some("RO"),
        191 => Some("CO"),
        193 => Some("HU"),
        194 => Some("AT"),
        195 => Some("GR"),
        196 => Some("UA"),
        197 => Some("NG"),
        214..=215 => Some("PH"),
        217 => Some("BE"),
        1..=4
        | 8
        | 13
        | 20..=24
        | 40
        | 44
        | 45
        | 47
        | 50
        | 52
        | 54
        | 63
        | 64
        | 65
        | 66
        | 67
        | 69
        | 71
        | 72
        | 73
        | 74
        | 75
        | 76
        | 96..=100
        | 104
        | 107
        | 108
        | 128..=150
        | 152..=186
        | 192
        | 198
        | 199
        | 204..=209
        | 216 => Some("US"),
        _ => None,
    }
}

pub fn country_flag(code: &str) -> String {
    if code.len() != 2 {
        return String::new();
    }

    let bytes = code.as_bytes();
    let a = bytes[0].to_ascii_uppercase();
    let b = bytes[1].to_ascii_uppercase();

    if !a.is_ascii_uppercase() || !b.is_ascii_uppercase() {
        return String::new();
    }

    format!(
        "{}{}",
        char::from_u32(0x1F1E6 + (a - b'A') as u32).unwrap_or(' '),
        char::from_u32(0x1F1E6 + (b - b'A') as u32).unwrap_or(' '),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        connection::{ConnectionConfig, ConnectionManager},
        dht::create_provider_cache,
    };

    fn dashboard_state() -> (DashboardState, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let (event_bus, _) = tokio::sync::broadcast::channel(16);
        let state = DashboardState {
            provider_cache: create_provider_cache(),
            reputation: Arc::new(Mutex::new(ReputationTracker::new(5))),
            conn_mgr: Arc::new(Mutex::new(ConnectionManager::new(ConnectionConfig::default()))),
            build_registry: Arc::new(Mutex::new(HashMap::new())),
            started: Instant::now(),
            peer_id: "local-peer".to_string(),
            event_bus,
            nar_store: Arc::new(Mutex::new(NarStore::new(tmp.path(), 262_144))),
            catalog: Arc::new(Mutex::new(HashMap::new())),
            cmd_tx: tokio::sync::mpsc::unbounded_channel().0,
            seed_mutation_allowed: true,
        };
        (state, tmp)
    }

    #[tokio::test]
    async fn build_detail_can_be_loaded_by_registry_key_or_nar_hash() {
        let (state, _tmp) = dashboard_state();
        let build = ObservedBuild {
            nar_hash: "sha256:abcdef".to_string(),
            store_path: Some("/gnu/store/hash-package".to_string()),
            nar_size: Some(42),
            references: vec![],
            deriver: None,
            narinfo_raw: None,
            providers: vec![],
            downloaded_at: None,
            download_size: None,
        };
        state.build_registry.lock().unwrap().insert("storehash".to_string(), build);

        let by_key =
            api_build_detail(State(state.clone()), Path("storehash".to_string())).await.unwrap().0;
        let by_nar_hash =
            api_build_detail(State(state), Path("sha256:abcdef".to_string())).await.unwrap().0;

        assert_eq!(by_key.nar_hash, "sha256:abcdef");
        assert_eq!(by_nar_hash.store_path.as_deref(), Some("/gnu/store/hash-package"));
    }

    #[tokio::test]
    async fn peers_api_returns_full_peer_ids_and_reputation_counters() {
        let (state, _tmp) = dashboard_state();
        let peer = libp2p::PeerId::random();
        {
            let mut reputation = state.reputation.lock().unwrap();
            reputation.record_success(peer, 4096);
            reputation.record_failure(peer);
        }

        let peers = api_peers(State(state)).await.0;

        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].peer_id, peer.to_string());
        assert_eq!(peers[0].completed, 1);
        assert_eq!(peers[0].failed, 1);
        assert_eq!(peers[0].bytes_served, 4096);
    }

    #[test]
    fn catalog_upsert_preserves_known_fields_and_latches_p2p_availability() {
        let catalog = Arc::new(Mutex::new(HashMap::new()));

        upsert_catalog_item(
            &catalog,
            "hashpart".to_string(),
            Some("/gnu/store/hash-package".to_string()),
            None,
            None,
            false,
        );
        upsert_catalog_item(
            &catalog,
            "hashpart".to_string(),
            None,
            Some(128),
            Some("sha256:abcdef".to_string()),
            true,
        );
        upsert_catalog_item(&catalog, "hashpart".to_string(), None, None, None, false);

        let guard = catalog.lock().unwrap();
        let item = guard.get("hashpart").unwrap();
        assert_eq!(item.store_path.as_deref(), Some("/gnu/store/hash-package"));
        assert_eq!(item.nar_size, Some(128));
        assert_eq!(item.nar_hash.as_deref(), Some("sha256:abcdef"));
        assert!(item.p2p_available);
    }

    #[test]
    fn installed_package_parser_reads_tab_separated_guix_output() {
        let seeded_store_paths = HashSet::from(["/gnu/store/hash-hello-2.12".to_string()]);
        let package = parse_installed_package(
            "system",
            "hello\t2.12\tout\t/gnu/store/hash-hello-2.12",
            &seeded_store_paths,
        )
        .unwrap();

        assert_eq!(
            package,
            ApiPackage {
                source: "system".to_string(),
                name: "hello".to_string(),
                version: "2.12".to_string(),
                output: "out".to_string(),
                store_path: "/gnu/store/hash-hello-2.12".to_string(),
                seeded: true,
            }
        );
    }

    #[test]
    fn package_profiles_include_system_kernel_and_home_sources() {
        let profiles = package_profiles_for_home(Some(PathBuf::from("/home/tester")));

        assert_eq!(
            profiles[0],
            ("system".to_string(), PathBuf::from("/run/current-system/profile"))
        );
        assert_eq!(
            profiles[1],
            ("kernel".to_string(), PathBuf::from("/run/current-system/kernel"))
        );
        assert_eq!(
            profiles[2],
            ("home".to_string(), PathBuf::from("/home/tester/.guix-home/profile"))
        );
    }

    #[test]
    fn missing_package_profiles_are_skipped_cleanly() {
        let seeded_store_paths = HashSet::new();
        let packages = installed_packages_from_profile(
            "missing",
            FsPath::new("/gnu/store/definitely-not-a-dashboard-test-profile"),
            &seeded_store_paths,
        );

        assert!(packages.is_empty());
    }

    #[test]
    fn seed_mutation_requires_loopback_bind_address() {
        assert!(seed_mutation_allowed_for_bind("127.0.0.1"));
        assert!(seed_mutation_allowed_for_bind("::1"));
        assert!(!seed_mutation_allowed_for_bind("0.0.0.0"));
        assert!(!seed_mutation_allowed_for_bind("192.168.1.111"));
    }

    #[tokio::test]
    async fn seed_api_rejects_non_store_paths() {
        let (state, _tmp) = dashboard_state();
        let result = api_seed(
            State(state),
            Json(ApiSeedRequest { store_path: "/tmp/not-store".to_string() }),
        )
        .await;

        assert_eq!(result.unwrap_err().0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn seed_api_rejects_mutation_when_dashboard_is_not_loopback() {
        let (mut state, _tmp) = dashboard_state();
        state.seed_mutation_allowed = false;
        let result = api_seed(
            State(state),
            Json(ApiSeedRequest { store_path: "/gnu/store/missing".to_string() }),
        )
        .await;

        assert_eq!(result.unwrap_err().0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn seed_path_persistence_deduplicates_existing_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(&config_path, "# user config\nseed_paths = [\"/gnu/store/aaaa-existing\"]\n")
            .unwrap();

        persist_seed_path_to_config("/gnu/store/bbbb-new", &config_path).unwrap();
        persist_seed_path_to_config("/gnu/store/bbbb-new", &config_path).unwrap();

        let content = fs::read_to_string(config_path).unwrap();
        assert!(content.contains("# user config"));
        assert_eq!(content.matches("/gnu/store/bbbb-new").count(), 1);
        assert!(content.contains("/gnu/store/aaaa-existing"));
    }

    #[test]
    fn seed_path_removal_updates_persisted_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            "# user config\nseed_paths = [\"/gnu/store/aaaa-keep\", \"/gnu/store/bbbb-remove\"]\n",
        )
        .unwrap();

        remove_seed_path_from_config("/gnu/store/bbbb-remove", &config_path).unwrap();

        let content = fs::read_to_string(config_path).unwrap();
        assert!(content.contains("# user config"));
        assert!(content.contains("/gnu/store/aaaa-keep"));
        assert!(!content.contains("/gnu/store/bbbb-remove"));
    }
}
