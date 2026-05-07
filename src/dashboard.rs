use std::{
    collections::HashMap,
    net::SocketAddr,
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
    routing::get,
};
use serde::Serialize;

use crate::{
    connection::ConnectionManager, dht::ProviderCache, nar_store::NarStore,
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
}

#[derive(Debug, Clone, Serialize)]
struct ApiCatalogEntry {
    hash_part: String,
    store_path: Option<String>,
    nar_size: Option<u64>,
    nar_hash: Option<String>,
    p2p_available: bool,
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
}

pub async fn serve(state: DashboardState, port: u16, bind: &str) {
    let addr: SocketAddr =
        format!("{}:{}", bind, port).parse().expect("invalid dashboard bind address");

    let app = Router::new()
        .route("/", get(index_html))
        .route("/api/status", get(api_status))
        .route("/api/peers", get(api_peers))
        .route("/api/builds", get(api_builds))
        .route("/api/build/{hash}", get(api_build_detail))
        .route("/api/catalog", get(api_catalog))
        .route("/api/seeds", get(api_seeds))
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
        .peers()
        .iter()
        .map(|(peer, score)| {
            let addrs = vec![];
            let (ip, country) = extract_addr_info(&addrs);
            ApiPeer {
                peer_id: peer.to_base58()[..16].to_string(),
                score: *score,
                completed: 0,
                failed: 0,
                bytes_served: 0,
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
        .values()
        .map(|b| ApiBuild {
            nar_hash: b.nar_hash.clone(),
            store_path: b.store_path.clone(),
            nar_size: b.nar_size,
            provider_count: b.providers.len(),
        })
        .collect();
    builds.sort_by_key(|b| std::cmp::Reverse(b.provider_count));
    Json(builds)
}

async fn api_catalog(State(state): State<DashboardState>) -> Json<Vec<ApiCatalogEntry>> {
    let cat = state.catalog.lock().unwrap();
    let entries: Vec<ApiCatalogEntry> = cat
        .values()
        .map(|c| ApiCatalogEntry {
            hash_part: c.hash_part.clone(),
            store_path: c.store_path.clone(),
            nar_size: c.nar_size,
            nar_hash: c.nar_hash.clone(),
            p2p_available: c.p2p_available,
        })
        .collect();
    Json(entries)
}

async fn api_seeds(State(state): State<DashboardState>) -> Json<Vec<ApiSeededNar>> {
    let store = state.nar_store.lock().unwrap();
    let seeds: Vec<ApiSeededNar> = store
        .seeded_hashes()
        .into_iter()
        .filter_map(|hash| {
            let info = store.seed_info(&hash)?;
            Some(ApiSeededNar {
                nar_hash: hash,
                nar_size: info.nar_size,
                block_count: info.block_count,
                block_size: info.block_size,
            })
        })
        .collect();
    Json(seeds)
}

async fn api_build_detail(
    State(state): State<DashboardState>,
    Path(hash): Path<String>,
) -> Result<Json<ObservedBuild>, StatusCode> {
    let reg = state.build_registry.lock().unwrap();
    reg.get(&hash).cloned().map(Json).ok_or(StatusCode::NOT_FOUND)
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

        if let DashboardEvent::CatalogEntry {
            ref hash_part,
            ref store_path,
            nar_size,
            ref nar_hash,
            p2p_available,
        } = event
        {
            let mut cat = state.catalog.lock().unwrap();
            cat.entry(hash_part.clone())
                .and_modify(|e: &mut CatalogItem| {
                    if store_path.is_some() {
                        e.store_path = store_path.clone();
                    }
                    if nar_size.is_some() {
                        e.nar_size = nar_size;
                    }
                    if nar_hash.is_some() {
                        e.nar_hash = nar_hash.clone();
                    }
                    if p2p_available {
                        e.p2p_available = true;
                    }
                })
                .or_insert_with(|| CatalogItem {
                    hash_part: hash_part.clone(),
                    store_path: store_path.clone(),
                    nar_size,
                    nar_hash: nar_hash.clone(),
                    p2p_available,
                });
        }

        let json = match serde_json::to_string(&event) {
            Ok(j) => j,
            Err(_) => continue,
        };

        if socket.send(Message::Text(json)).await.is_err() {
            break;
        }
    }
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
