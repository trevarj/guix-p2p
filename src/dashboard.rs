use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::{IpAddr, SocketAddr},
    path::Path as FsPath,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
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

use crate::{
    channel::SwarmCommand,
    config::Config,
    connection::{ConnectionManager, PeerConnectionSnapshot},
    dht::ProviderCache,
    diagnostics::{self, BootstrapBundle, ConnectivitySummary, DiagnosticReport},
    nar_store::NarStore,
    reputation::{PeerScore, ReputationTracker},
    version,
};

mod catalog;
mod geo;
mod packages;
mod seed_config;

pub use catalog::CatalogItem;
use catalog::upsert_catalog_item;
pub use geo::country_flag;
use geo::extract_addr_info;
use packages::{ApiPackage, installed_packages_from_profile, package_profiles};
use seed_config::{persist_seed_path_to_config, remove_seed_path_from_config, user_config_path};

pub type EventBus = tokio::sync::broadcast::Sender<DashboardEvent>;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum DashboardEvent {
    PeerDialStarted {
        peer_id: Option<String>,
    },
    PeerDialFailed {
        peer_id: Option<String>,
        reason: String,
    },
    PeerInboundStarted {
        address: String,
    },
    PeerInboundFailed {
        peer_id: Option<String>,
        address: String,
        reason: String,
    },
    PeerConnected {
        peer_id: String,
        addresses: Vec<String>,
    },
    PeerDisconnected {
        peer_id: String,
        reason: Option<String>,
    },
    ProvidersFound {
        nar_hash: String,
        provider_count: usize,
    },
    ProviderLookupStarted {
        nar_hash: String,
    },
    ProviderLookupFinished {
        nar_hash: String,
        provider_count: usize,
        result: String,
    },
    ProviderAnnounceStarted {
        nar_hash: String,
        reason: String,
    },
    ProviderAnnounceFinished {
        nar_hash: String,
        result: String,
        reason: Option<String>,
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
    TransferPhase {
        nar_hash: String,
        phase: String,
        elapsed_ms: u64,
    },
    BlockReceived {
        nar_hash: String,
        peer_id: String,
        indices: Vec<u32>,
        bytes: u64,
    },
    DownloadSucceeded {
        nar_hash: String,
        store_path: String,
        size: u64,
        elapsed_ms: u64,
        source: String,
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
        source: String,
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
pub type TransferRegistry = Arc<Mutex<HashMap<String, TransferStats>>>;
pub type EventHistoryRegistry = Arc<Mutex<EventHistory>>;

const EVENT_HISTORY_LIMIT: usize = 500;

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

#[derive(Debug, Clone, Default, Serialize)]
pub struct TransferStats {
    pub nar_hash: String,
    pub store_path: Option<String>,
    pub nar_size: Option<u64>,
    pub source: Option<String>,
    pub total_blocks_received: usize,
    pub total_bytes_received: u64,
    pub total_blocks_served: usize,
    pub download_peers: HashMap<String, PeerTransferStats>,
    pub serving_peers: HashMap<String, PeerTransferStats>,
    pub phases_ms: HashMap<String, u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PeerTransferStats {
    pub blocks: usize,
    pub bytes: u64,
    pub last_indices: Vec<u32>,
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
    address_count: usize,
    last_active_secs_ago: Option<u64>,
    country: Option<String>,
    ip: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ApiStatus {
    version: String,
    peer_id: String,
    listen_addr: String,
    external_addresses: Vec<String>,
    bootstrap_peers: Vec<String>,
    bootstrap_peer_count: usize,
    connectivity: ConnectivitySummary,
    shareable_addresses: Vec<String>,
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
    source: String,
    created_at: u64,
}

#[derive(Debug, Clone, Serialize)]
struct ApiCatalogEntry {
    hash_part: String,
    store_path: Option<String>,
    nar_size: Option<u64>,
    nar_hash: Option<String>,
    p2p_available: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ApiTransfer {
    nar_hash: String,
    store_path: Option<String>,
    nar_size: Option<u64>,
    source: Option<String>,
    total_blocks_received: usize,
    total_bytes_received: u64,
    total_blocks_served: usize,
    download_peers: Vec<ApiTransferPeer>,
    serving_peers: Vec<ApiTransferPeer>,
    phases_ms: HashMap<String, u64>,
}

#[derive(Debug, Clone, Serialize)]
struct ApiTransferPeer {
    peer_id: String,
    blocks: usize,
    bytes: u64,
    last_indices: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct ApiDashboardEvent {
    id: u64,
    timestamp_ms: u64,
    event: DashboardEvent,
}

#[derive(Debug, Default)]
pub struct EventHistory {
    next_id: u64,
    entries: VecDeque<ApiDashboardEvent>,
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

#[derive(Clone)]
pub struct DashboardState {
    pub provider_cache: ProviderCache,
    pub reputation: Arc<Mutex<ReputationTracker>>,
    pub conn_mgr: Arc<Mutex<ConnectionManager>>,
    pub build_registry: BuildRegistry,
    pub transfer_registry: TransferRegistry,
    pub event_history: EventHistoryRegistry,
    pub started: Instant,
    pub peer_id: String,
    pub listen_addr: String,
    pub external_addresses: Vec<String>,
    pub bootstrap_peers: Vec<String>,
    pub event_bus: EventBus,
    pub nar_store: Arc<Mutex<NarStore>>,
    pub catalog: Arc<Mutex<HashMap<String, CatalogItem>>>,
    pub cmd_tx: UnboundedSender<SwarmCommand>,
    pub seed_mutation_allowed: bool,
    pub config: Config,
}

pub async fn serve(state: DashboardState, port: u16, bind: &str) {
    let addr: SocketAddr =
        format!("{}:{}", bind, port).parse().expect("invalid dashboard bind address");

    let catalog_state = state.clone();
    tokio::spawn(async move {
        maintain_catalog(catalog_state).await;
    });

    let transfer_state = state.clone();
    tokio::spawn(async move {
        maintain_transfers(transfer_state).await;
    });

    let event_history_state = state.clone();
    tokio::spawn(async move {
        maintain_event_history(event_history_state).await;
    });

    let app = dashboard_router(state);

    tracing::info!("Dashboard listening on http://{}", addr);

    let listener =
        tokio::net::TcpListener::bind(addr).await.expect("failed to bind dashboard port");
    axum::serve(listener, app).await.expect("dashboard server error");
}

fn dashboard_router(state: DashboardState) -> Router {
    Router::new()
        .route("/", get(index_html))
        .route("/api/status", get(api_status))
        .route("/api/diagnostics", get(api_diagnostics))
        .route("/api/share-info", get(api_share_info))
        .route("/api/peers", get(api_peers))
        .route("/api/builds", get(api_builds))
        .route("/api/build/:hash", get(api_build_detail))
        .route("/api/transfers", get(api_transfers))
        .route("/api/events", get(api_events))
        .route("/api/catalog", get(api_catalog))
        .route("/api/seeds", get(api_seeds).post(api_seed))
        .route("/api/seeds/:hash", delete(api_seed_delete))
        .route("/api/packages", get(api_packages))
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn index_html() -> Html<&'static str> {
    Html(dashboard_html())
}

pub fn dashboard_html() -> &'static str {
    include_str!("dashboard.html")
}

async fn api_status(State(state): State<DashboardState>) -> Json<ApiStatus> {
    let now = state.started.elapsed().as_secs();
    let dht = state.provider_cache.lock().await.len();
    let builds = state.build_registry.lock().unwrap().len();
    let seed_count = state.nar_store.lock().unwrap().len();
    let connected = state.conn_mgr.lock().unwrap().connected_count();
    let connectivity = diagnostics::connectivity_summary_from_parts(
        &state.bootstrap_peers,
        &state.external_addresses,
        &state.peer_id,
    );

    let status = ApiStatus {
        version: version::VERSION.to_string(),
        peer_id: state.peer_id.clone(),
        listen_addr: state.listen_addr.clone(),
        external_addresses: state.external_addresses.clone(),
        bootstrap_peers: state.bootstrap_peers.clone(),
        bootstrap_peer_count: state.bootstrap_peers.len(),
        shareable_addresses: connectivity.shareable_addresses.clone(),
        connectivity,
        uptime_secs: now,
        connected_peers: connected,
        dht_entries: dht,
        build_count: builds,
        seed_count,
    };

    Json(status)
}

async fn api_diagnostics(State(state): State<DashboardState>) -> Json<DiagnosticReport> {
    Json(diagnostics::diagnostic_report(&state.config, &state.peer_id))
}

async fn api_share_info(State(state): State<DashboardState>) -> Json<BootstrapBundle> {
    Json(diagnostics::bootstrap_bundle(&state.config, &state.peer_id))
}

async fn api_peers(State(state): State<DashboardState>) -> Json<Vec<ApiPeer>> {
    let now = Instant::now();
    let rep_entries: HashMap<_, _> =
        state.reputation.lock().unwrap().peer_entries().into_iter().collect();
    let conn_entries: HashMap<_, _> =
        state.conn_mgr.lock().unwrap().peer_snapshots().into_iter().map(|s| (s.peer, s)).collect();

    let mut peer_ids: HashSet<_> = rep_entries.keys().copied().collect();
    peer_ids.extend(conn_entries.keys().copied());

    let mut peers: Vec<ApiPeer> = peer_ids
        .into_iter()
        .map(|peer| api_peer_from_parts(peer, rep_entries.get(&peer), conn_entries.get(&peer), now))
        .collect();
    peers.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.peer_id.cmp(&b.peer_id))
    });
    Json(peers)
}

fn api_peer_from_parts(
    peer: libp2p::PeerId,
    peer_score: Option<&PeerScore>,
    conn: Option<&PeerConnectionSnapshot>,
    now: Instant,
) -> ApiPeer {
    let addresses = conn.map(|snapshot| snapshot.addresses.clone()).unwrap_or_default();
    let (ip, country) = extract_addr_info(&addresses);
    ApiPeer {
        peer_id: peer.to_string(),
        score: peer_score.map(|score| score.score(now)).unwrap_or(0.5),
        completed: peer_score.map_or(0, |score| score.completed),
        failed: peer_score.map_or(0, |score| score.failed),
        bytes_served: peer_score.map_or(0, |score| score.bytes_served),
        connected: conn.is_some_and(|snapshot| snapshot.connected),
        address_count: addresses.len(),
        last_active_secs_ago: conn.map(|snapshot| snapshot.last_active_secs_ago),
        addresses,
        country,
        ip,
    }
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

async fn api_transfers(State(state): State<DashboardState>) -> Json<Vec<ApiTransfer>> {
    let transfers = state.transfer_registry.lock().unwrap();
    let mut entries: Vec<ApiTransfer> = transfers.values().map(api_transfer_from_stats).collect();
    entries.sort_by(|a, b| {
        b.total_blocks_received
            .cmp(&a.total_blocks_received)
            .then_with(|| b.total_blocks_served.cmp(&a.total_blocks_served))
            .then_with(|| a.store_path.cmp(&b.store_path))
            .then_with(|| a.nar_hash.cmp(&b.nar_hash))
    });
    Json(entries)
}

async fn api_events(State(state): State<DashboardState>) -> Json<Vec<ApiDashboardEvent>> {
    Json(state.event_history.lock().unwrap().entries.iter().cloned().collect())
}

fn api_transfer_from_stats(stats: &TransferStats) -> ApiTransfer {
    ApiTransfer {
        nar_hash: stats.nar_hash.clone(),
        store_path: stats.store_path.clone(),
        nar_size: stats.nar_size,
        source: stats.source.clone(),
        total_blocks_received: stats.total_blocks_received,
        total_bytes_received: stats.total_bytes_received,
        total_blocks_served: stats.total_blocks_served,
        download_peers: transfer_peer_entries(&stats.download_peers),
        serving_peers: transfer_peer_entries(&stats.serving_peers),
        phases_ms: stats.phases_ms.clone(),
    }
}

fn transfer_peer_entries(peers: &HashMap<String, PeerTransferStats>) -> Vec<ApiTransferPeer> {
    let mut entries: Vec<ApiTransferPeer> = peers
        .iter()
        .map(|(peer_id, stats)| ApiTransferPeer {
            peer_id: peer_id.clone(),
            blocks: stats.blocks,
            bytes: stats.bytes,
            last_indices: stats.last_indices.clone(),
        })
        .collect();
    entries.sort_by(|a, b| {
        b.blocks
            .cmp(&a.blocks)
            .then_with(|| b.bytes.cmp(&a.bytes))
            .then_with(|| a.peer_id.cmp(&b.peer_id))
    });
    entries
}

async fn api_seeds(State(state): State<DashboardState>) -> Json<Vec<ApiSeededNar>> {
    let known_store_paths = known_seed_store_paths(&state);
    let mut store = state.nar_store.lock().unwrap();
    let mut seeds: Vec<ApiSeededNar> = store
        .seeded_hashes()
        .into_iter()
        .filter_map(|hash| {
            if let Some(store_path) = known_store_paths.get(&hash) {
                store.annotate_seed_store_path(&hash, store_path);
            }
            let info = store.seed_info(&hash)?;
            Some(ApiSeededNar {
                nar_hash: hash,
                nar_size: info.nar_size,
                block_count: info.block_count,
                block_size: info.block_size,
                store_path: info.store_path,
                source: info.source.as_str().to_string(),
                created_at: info.created_at,
            })
        })
        .collect();
    seeds.sort_by(|a, b| b.nar_size.cmp(&a.nar_size).then_with(|| a.nar_hash.cmp(&b.nar_hash)));
    Json(seeds)
}

fn known_seed_store_paths(state: &DashboardState) -> HashMap<String, String> {
    let mut paths = HashMap::new();

    for build in state.build_registry.lock().unwrap().values() {
        if let Some(store_path) = &build.store_path {
            paths.insert(normalize_nar_hash(&build.nar_hash), store_path.clone());
        }
    }

    for item in state.catalog.lock().unwrap().values() {
        let (Some(nar_hash), Some(store_path)) = (&item.nar_hash, &item.store_path) else {
            continue;
        };
        paths.entry(normalize_nar_hash(nar_hash)).or_insert_with(|| store_path.clone());
    }

    paths
}

fn normalize_nar_hash(hash: &str) -> String {
    hash.strip_prefix("sha256:").unwrap_or(hash).to_string()
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
    .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("seed failed: {e:#}")))?;

    let info = state
        .nar_store
        .lock()
        .unwrap()
        .seed_info(&nar_hash)
        .ok_or_else(|| api_error(StatusCode::INTERNAL_SERVER_ERROR, "seed metadata missing"))?;

    complete_seed_mutation(&state, &store_path, &nar_hash, info.nar_size, &user_config_path())?;

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

    let info = state.nar_store.lock().unwrap().remove_seed(&hash);
    let store_path = info.and_then(|info| info.store_path);

    complete_seed_removal(&state, &hash, store_path, &user_config_path());

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

fn complete_seed_mutation(
    state: &DashboardState,
    store_path: &str,
    nar_hash: &str,
    nar_size: u64,
    config_path: &FsPath,
) -> Result<(), (StatusCode, Json<ApiError>)> {
    state.cmd_tx.send(SwarmCommand::StartProviding { hash: nar_hash.to_string() }).map_err(
        |e| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("failed to announce seed: {e}")),
    )?;

    persist_seed_path_to_config(store_path, config_path).map_err(|e| {
        api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("persist failed: {e}"))
    })?;

    let _ = state.event_bus.send(DashboardEvent::SeedAdded {
        nar_hash: nar_hash.to_string(),
        store_path: Some(store_path.to_string()),
        nar_size,
        source: "manual".to_string(),
    });

    Ok(())
}

fn complete_seed_removal(
    state: &DashboardState,
    nar_hash: &str,
    store_path: Option<String>,
    config_path: &FsPath,
) {
    if let Some(path) = &store_path
        && let Err(e) = remove_seed_path_from_config(path, config_path)
    {
        tracing::warn!("failed to remove seed path from config: {}", e);
    }

    let _ = state
        .event_bus
        .send(DashboardEvent::SeedRemoved { nar_hash: nar_hash.to_string(), store_path });
}

pub fn seed_mutation_allowed_for_bind(bind: &str) -> bool {
    bind.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
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

async fn maintain_transfers(state: DashboardState) {
    let mut rx = state.event_bus.subscribe();

    loop {
        let event = match rx.recv().await {
            Ok(e) => e,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::debug!("Dashboard transfer listener lagged by {} events", n);
                continue;
            },
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };

        match event {
            DashboardEvent::DownloadStarted { nar_hash, store_path, nar_size } => {
                let mut transfers = state.transfer_registry.lock().unwrap();
                let entry = transfers
                    .entry(nar_hash.clone())
                    .or_insert_with(|| TransferStats { nar_hash, ..TransferStats::default() });
                entry.store_path = Some(store_path);
                entry.nar_size = Some(nar_size);
            },
            DashboardEvent::BlockReceived { nar_hash, peer_id, indices, bytes } => {
                let mut transfers = state.transfer_registry.lock().unwrap();
                let entry = transfers
                    .entry(nar_hash.clone())
                    .or_insert_with(|| TransferStats { nar_hash, ..TransferStats::default() });
                entry.total_blocks_received += indices.len();
                entry.total_bytes_received += bytes;
                record_transfer_peer(&mut entry.download_peers, peer_id, indices, bytes);
            },
            DashboardEvent::TransferPhase { nar_hash, phase, elapsed_ms } => {
                let mut transfers = state.transfer_registry.lock().unwrap();
                let entry = transfers
                    .entry(nar_hash.clone())
                    .or_insert_with(|| TransferStats { nar_hash, ..TransferStats::default() });
                entry.phases_ms.insert(phase, elapsed_ms);
            },
            DashboardEvent::BlockServed { nar_hash, peer_id, indices } => {
                if indices.is_empty() {
                    continue;
                }
                let mut transfers = state.transfer_registry.lock().unwrap();
                let entry = transfers
                    .entry(nar_hash.clone())
                    .or_insert_with(|| TransferStats { nar_hash, ..TransferStats::default() });
                entry.total_blocks_served += indices.len();
                record_transfer_peer(&mut entry.serving_peers, peer_id, indices, 0);
            },
            DashboardEvent::DownloadSucceeded { nar_hash, store_path, size, source, .. } => {
                let mut transfers = state.transfer_registry.lock().unwrap();
                let entry = transfers
                    .entry(nar_hash.clone())
                    .or_insert_with(|| TransferStats { nar_hash, ..TransferStats::default() });
                entry.store_path = Some(store_path);
                entry.nar_size = Some(size);
                entry.source = Some(source);
            },
            _ => {},
        }
    }
}

async fn maintain_event_history(state: DashboardState) {
    let mut rx = state.event_bus.subscribe();

    loop {
        let event = match rx.recv().await {
            Ok(e) => e,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::debug!("Dashboard event-history listener lagged by {} events", n);
                continue;
            },
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };

        record_event_history(&state.event_history, event);
    }
}

fn record_event_history(history: &EventHistoryRegistry, event: DashboardEvent) {
    let mut history = history.lock().unwrap();
    let id = history.next_id;
    history.next_id += 1;
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    history.entries.push_back(ApiDashboardEvent { id, timestamp_ms, event });
    while history.entries.len() > EVENT_HISTORY_LIMIT {
        history.entries.pop_front();
    }
}

fn record_transfer_peer(
    peers: &mut HashMap<String, PeerTransferStats>,
    peer_id: String,
    indices: Vec<u32>,
    bytes: u64,
) {
    let peer = peers.entry(peer_id).or_default();
    peer.blocks += indices.len();
    peer.bytes += bytes;
    peer.last_indices = indices;
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use axum::http::{Method, Request};
    use sha2::Digest;
    use tower::ServiceExt;

    use super::{
        packages::{package_profiles_for_home, parse_installed_package},
        *,
    };
    use crate::{
        connection::{ConnectionConfig, ConnectionManager},
        dht::create_provider_cache,
    };

    fn dashboard_state() -> (DashboardState, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let (event_bus, _) = tokio::sync::broadcast::channel(16);
        let mut config =
            Config::load(None, None, None, Some(tmp.path().display().to_string()), None, None);
        config.external_addresses = vec!["/dns4/node.example.org/udp/6881/quic-v1".to_string()];
        config.bootstrap_peers =
            vec!["/dns4/bootstrap.example.org/udp/6881/quic-v1/p2p/12D3KooWQp4D6Lwq".to_string()];
        let state = DashboardState {
            provider_cache: create_provider_cache(),
            reputation: Arc::new(Mutex::new(ReputationTracker::new(5))),
            conn_mgr: Arc::new(Mutex::new(ConnectionManager::new(ConnectionConfig::default()))),
            build_registry: Arc::new(Mutex::new(HashMap::new())),
            transfer_registry: Arc::new(Mutex::new(HashMap::new())),
            event_history: Arc::new(Mutex::new(EventHistory::default())),
            started: Instant::now(),
            peer_id: "local-peer".to_string(),
            listen_addr: "/ip4/0.0.0.0/udp/6881/quic-v1".to_string(),
            external_addresses: vec!["/dns4/node.example.org/udp/6881/quic-v1".to_string()],
            bootstrap_peers: vec![
                "/dns4/bootstrap.example.org/udp/6881/quic-v1/p2p/12D3KooWQp4D6Lwq".to_string(),
            ],
            event_bus,
            nar_store: Arc::new(Mutex::new(NarStore::new(tmp.path(), 262_144))),
            catalog: Arc::new(Mutex::new(HashMap::new())),
            cmd_tx: tokio::sync::mpsc::unbounded_channel().0,
            seed_mutation_allowed: true,
            config,
        };
        (state, tmp)
    }

    #[test]
    fn embedded_dashboard_keeps_refined_ux_contract() {
        let html = dashboard_html();
        for expected in [
            "aria-label=\"guix-p2p operational dashboard\"",
            "aria-label=\"Open node details\"",
            "role=\"dialog\" aria-modal=\"true\" aria-labelledby=\"detail-title\"",
            "<h2 id=\"detail-title\">",
            "aria-label=\"'+attr('Open transfer evidence for ",
            "aria-label=\"'+attr('Open seed detail for ",
            "aria-label=\"'+attr('Open discovery detail for ",
            "openDetailPanel",
            "trapDetailFocus",
            "DETAIL_RETURN_FOCUS",
            "open evidence &gt;",
            "stop seeding this nar",
            "metadata pending",
            "prefers-reduced-motion",
            "list-head",
            "transfer-progress",
        ] {
            assert!(html.contains(expected), "dashboard html missing {expected}");
        }
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
        assert!(!peers[0].connected);
        assert_eq!(peers[0].address_count, 0);
    }

    #[tokio::test]
    async fn status_api_returns_shareable_addresses() {
        let (mut state, _tmp) = dashboard_state();
        let peer = libp2p::PeerId::random();
        state.peer_id = peer.to_string();

        let status = api_status(State(state)).await.0;

        assert_eq!(status.listen_addr, "/ip4/0.0.0.0/udp/6881/quic-v1");
        assert_eq!(status.external_addresses, vec!["/dns4/node.example.org/udp/6881/quic-v1"]);
        assert_eq!(
            status.shareable_addresses,
            vec![format!("/dns4/node.example.org/udp/6881/quic-v1/p2p/{peer}")]
        );
        assert_eq!(status.bootstrap_peer_count, 1);
        assert_eq!(status.connectivity.state, "shareable");
    }

    #[tokio::test]
    async fn diagnostics_api_returns_doctor_report() {
        let (mut state, _tmp) = dashboard_state();
        let peer = libp2p::PeerId::random();
        state.peer_id = peer.to_string();

        let report = api_diagnostics(State(state)).await.0;

        assert_eq!(report.peer_id, peer.to_string());
        assert_eq!(report.connectivity.state, "shareable");
        assert!(report.checks.iter().any(|check| check.id == "bootstrap-peers"));
        assert!(report.checks.iter().any(|check| check.id == "shareable-address"));
    }

    #[tokio::test]
    async fn share_info_api_returns_bootstrap_bundle() {
        let (mut state, _tmp) = dashboard_state();
        let peer = libp2p::PeerId::random();
        state.peer_id = peer.to_string();

        let bundle = api_share_info(State(state)).await.0;

        assert_eq!(bundle.peer_id, peer.to_string());
        assert_eq!(
            bundle.bootstrap_peers,
            vec![format!("/dns4/node.example.org/udp/6881/quic-v1/p2p/{peer}")]
        );
        assert!(bundle.config_snippet.contains("bootstrap_peers ="));
    }

    #[tokio::test]
    async fn peers_api_includes_connected_peers_without_reputation() {
        let (state, _tmp) = dashboard_state();
        let peer = libp2p::PeerId::random();
        let address = "/ip4/127.0.0.1/tcp/6881".to_string();
        state.conn_mgr.lock().unwrap().on_connected_with_addresses(peer, vec![address.clone()]);

        let peers = api_peers(State(state)).await.0;

        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].peer_id, peer.to_string());
        assert!(peers[0].connected);
        assert_eq!(peers[0].score, 0.5);
        assert_eq!(peers[0].addresses, vec![address]);
        assert_eq!(peers[0].address_count, 1);
        assert_eq!(peers[0].last_active_secs_ago, Some(0));
    }

    #[tokio::test]
    async fn peers_api_merges_connection_snapshot_with_reputation() {
        let (state, _tmp) = dashboard_state();
        let peer = libp2p::PeerId::random();
        {
            let mut reputation = state.reputation.lock().unwrap();
            reputation.record_success(peer, 4096);
        }
        state
            .conn_mgr
            .lock()
            .unwrap()
            .on_connected_with_addresses(peer, vec!["/ip4/127.0.0.1/tcp/6881".to_string()]);

        let peers = api_peers(State(state)).await.0;

        assert_eq!(peers.len(), 1);
        assert!(peers[0].connected);
        assert_eq!(peers[0].completed, 1);
        assert_eq!(peers[0].bytes_served, 4096);
        assert_eq!(peers[0].address_count, 1);
    }

    #[tokio::test]
    async fn transfers_api_returns_peer_contribution_evidence() {
        let (state, _tmp) = dashboard_state();
        {
            let mut transfers = state.transfer_registry.lock().unwrap();
            transfers.insert(
                "sha256:transfer".to_string(),
                TransferStats {
                    nar_hash: "sha256:transfer".to_string(),
                    store_path: Some("/gnu/store/hash-package".to_string()),
                    nar_size: Some(8192),
                    source: Some("p2p-connected-fallback".to_string()),
                    total_blocks_received: 3,
                    total_bytes_received: 6144,
                    total_blocks_served: 2,
                    phases_ms: HashMap::from([
                        ("provider_lookup".to_string(), 120),
                        ("p2p_download".to_string(), 340),
                    ]),
                    download_peers: HashMap::from([
                        (
                            "peer-b".to_string(),
                            PeerTransferStats { blocks: 1, bytes: 2048, last_indices: vec![1] },
                        ),
                        (
                            "peer-a".to_string(),
                            PeerTransferStats { blocks: 2, bytes: 4096, last_indices: vec![0, 2] },
                        ),
                    ]),
                    serving_peers: HashMap::from([(
                        "peer-c".to_string(),
                        PeerTransferStats { blocks: 2, bytes: 0, last_indices: vec![3, 4] },
                    )]),
                },
            );
        }

        let transfers = api_transfers(State(state)).await.0;

        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].nar_hash, "sha256:transfer");
        assert_eq!(transfers[0].source.as_deref(), Some("p2p-connected-fallback"));
        assert_eq!(transfers[0].download_peers[0].peer_id, "peer-a");
        assert_eq!(transfers[0].download_peers[0].last_indices, vec![0, 2]);
        assert_eq!(transfers[0].download_peers[1].peer_id, "peer-b");
        assert_eq!(transfers[0].serving_peers[0].peer_id, "peer-c");
        assert_eq!(transfers[0].phases_ms["provider_lookup"], 120);
    }

    #[tokio::test]
    async fn events_api_returns_persisted_event_history() {
        let (state, _tmp) = dashboard_state();
        record_event_history(
            &state.event_history,
            DashboardEvent::PeerConnected {
                peer_id: "peer-a".to_string(),
                addresses: vec!["/ip4/127.0.0.1/tcp/1234".to_string()],
            },
        );
        record_event_history(
            &state.event_history,
            DashboardEvent::DownloadFailed {
                nar_hash: "sha256:abc".to_string(),
                store_path: "/gnu/store/hash-package".to_string(),
                reason: "missing providers".to_string(),
            },
        );

        let events = api_events(State(state)).await.0;

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, 0);
        assert_eq!(events[1].id, 1);
        assert!(events[1].timestamp_ms >= events[0].timestamp_ms);
        match &events[1].event {
            DashboardEvent::DownloadFailed { reason, .. } => {
                assert_eq!(reason, "missing providers");
            },
            other => panic!("unexpected dashboard event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn events_api_returns_connection_attempt_history() {
        let (state, _tmp) = dashboard_state();
        record_event_history(
            &state.event_history,
            DashboardEvent::PeerDialStarted { peer_id: Some("peer-a".to_string()) },
        );
        record_event_history(
            &state.event_history,
            DashboardEvent::PeerDialFailed {
                peer_id: Some("peer-a".to_string()),
                reason: "transport error".to_string(),
            },
        );
        record_event_history(
            &state.event_history,
            DashboardEvent::PeerInboundFailed {
                peer_id: None,
                address: "/ip4/198.51.100.10/tcp/1234".to_string(),
                reason: "handshake failed".to_string(),
            },
        );

        let events = api_events(State(state)).await.0;

        assert_eq!(events.len(), 3);
        match &events[1].event {
            DashboardEvent::PeerDialFailed { reason, .. } => {
                assert_eq!(reason, "transport error");
            },
            other => panic!("unexpected dashboard event: {other:?}"),
        }
        match &events[2].event {
            DashboardEvent::PeerInboundFailed { address, .. } => {
                assert_eq!(address, "/ip4/198.51.100.10/tcp/1234");
            },
            other => panic!("unexpected dashboard event: {other:?}"),
        }
    }

    #[test]
    fn event_history_keeps_newest_500_entries() {
        let history = Arc::new(Mutex::new(EventHistory::default()));
        for idx in 0..(EVENT_HISTORY_LIMIT + 3) {
            record_event_history(
                &history,
                DashboardEvent::PeerDisconnected { peer_id: format!("peer-{idx}"), reason: None },
            );
        }

        let guard = history.lock().unwrap();

        assert_eq!(guard.entries.len(), EVENT_HISTORY_LIMIT);
        assert_eq!(guard.entries.front().unwrap().id, 3);
        assert_eq!(guard.entries.back().unwrap().id, 502);
    }

    #[tokio::test]
    async fn packages_api_returns_sorted_snapshot() {
        let (state, _tmp) = dashboard_state();

        let packages = api_packages(State(state)).await.0;

        let mut sorted = packages.clone();
        sorted.sort_by(|a, b| {
            a.source
                .cmp(&b.source)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.version.cmp(&b.version))
                .then_with(|| a.output.cmp(&b.output))
                .then_with(|| a.store_path.cmp(&b.store_path))
        });
        assert_eq!(packages, sorted);
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
    fn dashboard_package_discovery_has_guix_command() {
        let command = packages::guix_binary();

        assert!(!command.is_empty());
    }

    #[test]
    fn dashboard_package_discovery_prefers_system_profile_guix() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("guix"), b"").unwrap();

        let command = packages::guix_binary_from_system_profile(tmp.path());

        assert_eq!(command, bin.join("guix").into_os_string());
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

    #[tokio::test]
    async fn seed_delete_api_removes_cached_seed_without_store_metadata() {
        let (state, _tmp) = dashboard_state();
        let nar_hash = "abcd".repeat(16);
        {
            let mut store = state.nar_store.lock().unwrap();
            store.save_with_store_path(&nar_hash, b"nar bytes", None).unwrap();
            assert!(store.has_nar(&nar_hash));
        }

        let status = api_seed_delete(State(state.clone()), Path(nar_hash.clone())).await.unwrap();

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(!state.nar_store.lock().unwrap().has_nar(&nar_hash));
    }

    #[tokio::test]
    async fn seed_delete_api_removes_seed_with_store_metadata_and_emits_event() {
        let (state, _tmp) = dashboard_state();
        let mut event_rx = state.event_bus.subscribe();
        let nar_hash = "abcd".repeat(16);
        let store_path = "/gnu/store/abcd-package";
        {
            let mut store = state.nar_store.lock().unwrap();
            store
                .save_with_store_path(&nar_hash, b"nar bytes", Some(store_path.to_string()))
                .unwrap();
            assert!(store.has_nar(&nar_hash));
        }

        let status = api_seed_delete(State(state.clone()), Path(nar_hash.clone())).await.unwrap();

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(!state.nar_store.lock().unwrap().has_nar(&nar_hash));
        match event_rx.recv().await.unwrap() {
            DashboardEvent::SeedRemoved { nar_hash: event_hash, store_path: event_path } => {
                assert_eq!(event_hash, nar_hash);
                assert_eq!(event_path.as_deref(), Some(store_path));
            },
            other => panic!("unexpected dashboard event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn seed_delete_api_treats_missing_seed_as_already_removed() {
        let (state, _tmp) = dashboard_state();

        let status = api_seed_delete(State(state), Path("abcd".repeat(16))).await.unwrap();

        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn seeds_api_labels_cached_nar_from_build_metadata() {
        let (state, tmp) = dashboard_state();
        let nar_data = b"nar bytes";
        let nar_hash = hex::encode(sha2::Sha256::digest(nar_data));
        let store_path = "/gnu/store/abcd-package";
        let nar_dir = tmp.path().join("nar");
        std::fs::create_dir_all(&nar_dir).unwrap();
        std::fs::write(nar_dir.join(format!("{nar_hash}.nar")), nar_data).unwrap();
        *state.nar_store.lock().unwrap() = NarStore::new(tmp.path(), 262144);
        state.build_registry.lock().unwrap().insert(
            "abcd-package".to_string(),
            ObservedBuild {
                nar_hash: nar_hash.clone(),
                store_path: Some(store_path.to_string()),
                nar_size: Some(9),
                references: vec![],
                deriver: None,
                narinfo_raw: None,
                providers: vec![],
                downloaded_at: None,
                download_size: None,
            },
        );

        let Json(seeds) = api_seeds(State(state.clone())).await;

        assert_eq!(seeds[0].store_path.as_deref(), Some(store_path));
        assert_eq!(
            state.nar_store.lock().unwrap().seed_info(&nar_hash).unwrap().store_path.as_deref(),
            Some(store_path)
        );
    }

    #[tokio::test]
    async fn seed_delete_route_matches_hash_path() {
        let (state, _tmp) = dashboard_state();
        let app = dashboard_router(state);
        let hash = "abcd".repeat(16);
        let request = Request::builder()
            .method(Method::DELETE)
            .uri(format!("/api/seeds/{hash}"))
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn seed_removal_ignores_config_cleanup_failures_after_local_remove() {
        let (state, tmp) = dashboard_state();
        let config_path = tmp.path().join("missing-parent/config.toml");
        let nar_hash = "abcd".repeat(16);
        let store_path = Some("/gnu/store/abcd-package".to_string());

        complete_seed_removal(&state, &nar_hash, store_path, &config_path);
    }

    #[tokio::test]
    async fn seed_completion_announces_emits_and_persists_config() {
        let (mut state, tmp) = dashboard_state();
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        state.cmd_tx = cmd_tx;
        let mut event_rx = state.event_bus.subscribe();
        let config_path = tmp.path().join("config.toml");
        let nar_hash = "abcd".repeat(16);
        let store_path = "/gnu/store/abcd-package";

        complete_seed_mutation(&state, store_path, &nar_hash, 42, &config_path).unwrap();

        match cmd_rx.recv().await.unwrap() {
            SwarmCommand::StartProviding { hash } => assert_eq!(hash, nar_hash),
            other => panic!("unexpected swarm command: {other:?}"),
        }
        match event_rx.recv().await.unwrap() {
            DashboardEvent::SeedAdded {
                nar_hash: event_hash,
                store_path: event_path,
                nar_size,
                source,
            } => {
                assert_eq!(event_hash, nar_hash);
                assert_eq!(event_path.as_deref(), Some(store_path));
                assert_eq!(nar_size, 42);
                assert_eq!(source, "manual");
            },
            other => panic!("unexpected dashboard event: {other:?}"),
        }
        let content = fs::read_to_string(config_path).unwrap();
        assert!(content.contains(store_path));
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
