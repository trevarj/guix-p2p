use std::{net::SocketAddr, time::Duration};

use axum::{
    Router,
    extract::{
        Path,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{delete, get},
};
use serde_json::{Value, json};

pub async fn serve(bind: &str, port: u16) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{bind}:{port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Demo dashboard listening on http://{}", addr);
    axum::serve(listener, router()).await?;
    Ok(())
}

pub fn router() -> Router {
    Router::new()
        .route("/", get(index_html))
        .route("/api/status", get(status))
        .route("/api/diagnostics", get(diagnostics))
        .route("/api/share-info", get(share_info))
        .route("/api/peers", get(peers))
        .route("/api/builds", get(builds))
        .route("/api/build/:hash", get(build_detail))
        .route("/api/transfers", get(transfers))
        .route("/api/events", get(events))
        .route("/api/catalog", get(catalog))
        .route("/api/seeds", get(seeds).post(seed))
        .route("/api/seeds/:hash", delete(seed_delete))
        .route("/api/packages", get(packages))
        .route("/ws", get(ws_handler))
}

async fn index_html() -> Html<&'static str> {
    Html(guix_p2p::dashboard::dashboard_html())
}

async fn status() -> Json<Value> {
    Json(json!({
        "peer_id": "12D3KooWDemoBobFetchNode",
        "listen_addr": "/ip4/0.0.0.0/udp/6883/quic-v1",
        "external_addresses": ["/ip4/127.0.0.1/udp/6883/quic-v1"],
        "bootstrap_peer_count": 1,
        "connectivity": {
            "state": "shareable",
            "detail": "bootstrap peers and shareable addresses are configured",
            "has_bootstrap_peers": true,
            "has_shareable_addresses": true,
            "has_private_external_addresses": true,
            "shareable_addresses": ["/ip4/127.0.0.1/udp/6883/quic-v1/p2p/12D3KooWDemoBobFetchNode"]
        },
        "shareable_addresses": ["/ip4/127.0.0.1/udp/6883/quic-v1/p2p/12D3KooWDemoBobFetchNode"],
        "uptime_secs": 1847,
        "connected_peers": 3,
        "dht_entries": 14,
        "build_count": 4,
        "seed_count": 2,
        "demo": true
    }))
}

async fn diagnostics() -> Json<Value> {
    Json(json!({
        "peer_id": "12D3KooWDemoBobFetchNode",
        "connectivity": {
            "state": "shareable",
            "detail": "bootstrap peers and shareable addresses are configured",
            "has_bootstrap_peers": true,
            "has_shareable_addresses": true,
            "has_private_external_addresses": true,
            "shareable_addresses": ["/ip4/127.0.0.1/udp/6883/quic-v1/p2p/12D3KooWDemoBobFetchNode"]
        },
        "has_errors": false,
        "has_warnings": true,
        "checks": [
            {
                "id": "identity",
                "severity": "ok",
                "summary": "identity loaded",
                "detail": "peer id: 12D3KooWDemoBobFetchNode"
            },
            {
                "id": "listen-address",
                "severity": "ok",
                "summary": "listen address is valid",
                "detail": "/ip4/0.0.0.0/udp/6883/quic-v1"
            },
            {
                "id": "bootstrap-peers",
                "severity": "ok",
                "summary": "bootstrap peers configured",
                "detail": "1 bootstrap peer(s)"
            },
            {
                "id": "shareable-address",
                "severity": "ok",
                "summary": "shareable peer address available",
                "detail": "/ip4/127.0.0.1/udp/6883/quic-v1/p2p/12D3KooWDemoBobFetchNode"
            },
            {
                "id": "nat-address",
                "severity": "warning",
                "summary": "external address looks private or loopback",
                "detail": "demo dashboard uses loopback addresses"
            }
        ]
    }))
}

async fn share_info() -> Json<Value> {
    Json(json!({
        "peer_id": "12D3KooWDemoBobFetchNode",
        "connectivity": {
            "state": "shareable",
            "detail": "bootstrap peers and shareable addresses are configured",
            "has_bootstrap_peers": true,
            "has_shareable_addresses": true,
            "has_private_external_addresses": true,
            "shareable_addresses": ["/ip4/127.0.0.1/udp/6883/quic-v1/p2p/12D3KooWDemoBobFetchNode"]
        },
        "shareable_addresses": ["/ip4/127.0.0.1/udp/6883/quic-v1/p2p/12D3KooWDemoBobFetchNode"],
        "bootstrap_peers": ["/ip4/127.0.0.1/udp/6883/quic-v1/p2p/12D3KooWDemoBobFetchNode"],
        "dashboard_url": null,
        "has_errors": false,
        "has_warnings": true,
        "config_snippet": "bootstrap_peers = \"/ip4/127.0.0.1/udp/6883/quic-v1/p2p/12D3KooWDemoBobFetchNode\""
    }))
}

async fn peers() -> Json<Value> {
    Json(json!([
        {
            "peer_id": "12D3KooWAliceSeedNode",
            "score": 0.94,
            "completed": 18,
            "failed": 1,
            "bytes_served": 84213760u64,
            "connected": true,
            "addresses": ["/ip4/127.0.0.1/tcp/6881"],
            "address_count": 1,
            "last_active_secs_ago": 3,
            "country": "US",
            "ip": "127.0.0.1"
        },
        {
            "peer_id": "12D3KooWCharlesSeedNode",
            "score": 0.87,
            "completed": 12,
            "failed": 0,
            "bytes_served": 51773440u64,
            "connected": true,
            "addresses": ["/ip4/127.0.0.1/tcp/6882"],
            "address_count": 1,
            "last_active_secs_ago": 8,
            "country": "US",
            "ip": "127.0.0.1"
        },
        {
            "peer_id": "12D3KooWBootstrapNode",
            "score": 0.5,
            "completed": 0,
            "failed": 0,
            "bytes_served": 0,
            "connected": true,
            "addresses": ["/ip4/127.0.0.1/tcp/6879"],
            "address_count": 1,
            "last_active_secs_ago": 2,
            "country": "US",
            "ip": "127.0.0.1"
        },
        {
            "peer_id": "12D3KooWFormerPeer",
            "score": 0.31,
            "completed": 2,
            "failed": 3,
            "bytes_served": 1048576u64,
            "connected": false,
            "addresses": [],
            "address_count": 0,
            "last_active_secs_ago": 940,
            "country": null,
            "ip": null
        }
    ]))
}

async fn builds() -> Json<Value> {
    Json(json!([
        {
            "lookup_key": "1xq2-demo-hello",
            "nar_hash": demo_hash(),
            "store_path": "/gnu/store/1xq2v3demohello-hello-2.12.1",
            "nar_size": 712704,
            "provider_count": 2
        },
        {
            "lookup_key": "8mc4-demo-guile",
            "nar_hash": "sha256:guile-demo-nar-hash",
            "store_path": "/gnu/store/8mc4v3demoguile-guile-3.0.9",
            "nar_size": 14860288,
            "provider_count": 1
        }
    ]))
}

async fn build_detail(Path(hash): Path<String>) -> Result<Json<Value>, StatusCode> {
    if hash != "1xq2-demo-hello" && hash != demo_hash() {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(json!({
        "nar_hash": demo_hash(),
        "store_path": "/gnu/store/1xq2v3demohello-hello-2.12.1",
        "nar_size": 712704,
        "references": [
            "/gnu/store/r2demo-glibc-2.39",
            "/gnu/store/r3demo-gcc-lib-13.3.0"
        ],
        "deriver": "/gnu/store/1xq2v3demohello-hello-2.12.1.drv",
        "narinfo_raw": "StorePath: /gnu/store/1xq2v3demohello-hello-2.12.1\nURL: nar/demo-hello.nar.xz\nNarHash: sha256:demo\nNarSize: 712704\nSignature: demo-only",
        "providers": ["12D3KooWAliceSeedNode", "12D3KooWCharlesSeedNode"],
        "downloaded_at": 1778841600u64,
        "download_size": 712704
    })))
}

async fn transfers() -> Json<Value> {
    Json(json!([
        {
            "nar_hash": demo_hash(),
            "store_path": "/gnu/store/1xq2v3demohello-hello-2.12.1",
            "nar_size": 712704,
            "total_blocks_received": 11,
            "total_bytes_received": 712704,
            "total_blocks_served": 5,
            "download_peers": [
                {
                    "peer_id": "12D3KooWAliceSeedNode",
                    "blocks": 7,
                    "bytes": 458752,
                    "last_indices": [6, 7, 8]
                },
                {
                    "peer_id": "12D3KooWCharlesSeedNode",
                    "blocks": 4,
                    "bytes": 253952,
                    "last_indices": [9, 10]
                }
            ],
            "serving_peers": [
                {
                    "peer_id": "12D3KooWCarolFetchNode",
                    "blocks": 5,
                    "bytes": 0,
                    "last_indices": [0, 1, 2, 3, 4]
                }
            ]
        }
    ]))
}

async fn events() -> Json<Value> {
    let base = 1_778_841_600_000u64;
    Json(json!([
        {"id": 0, "timestamp_ms": base, "event": {"type": "PeerDialStarted", "peer_id": "12D3KooWBootstrapNode"}},
        {"id": 1, "timestamp_ms": base + 1000, "event": {"type": "PeerConnected", "peer_id": "12D3KooWBootstrapNode", "addresses": ["/ip4/127.0.0.1/tcp/6879"]}},
        {"id": 2, "timestamp_ms": base + 2000, "event": {"type": "PeerDialFailed", "peer_id": "12D3KooWFormerPeer", "reason": "transport error: connection refused"}},
        {"id": 3, "timestamp_ms": base + 3000, "event": {"type": "PeerInboundStarted", "address": "/ip4/127.0.0.1/tcp/6884"}},
        {"id": 4, "timestamp_ms": base + 4000, "event": {"type": "PeerInboundFailed", "peer_id": null, "address": "/ip4/127.0.0.1/tcp/6884", "reason": "handshake failed"}},
        {"id": 5, "timestamp_ms": base + 5000, "event": {"type": "PeerConnected", "peer_id": "12D3KooWAliceSeedNode", "addresses": ["/ip4/127.0.0.1/tcp/6881"]}},
        {"id": 6, "timestamp_ms": base + 6000, "event": {"type": "BuildDiscovered", "nar_hash": demo_hash(), "store_path": "/gnu/store/1xq2v3demohello-hello-2.12.1", "nar_size": 712704}},
        {"id": 7, "timestamp_ms": base + 7000, "event": {"type": "ProvidersFound", "nar_hash": demo_hash(), "provider_count": 2}},
        {"id": 8, "timestamp_ms": base + 8000, "event": {"type": "DownloadStarted", "nar_hash": demo_hash(), "store_path": "/gnu/store/1xq2v3demohello-hello-2.12.1", "nar_size": 712704}},
        {"id": 9, "timestamp_ms": base + 9000, "event": {"type": "BlockReceived", "nar_hash": demo_hash(), "peer_id": "12D3KooWAliceSeedNode", "indices": [0,1,2], "bytes": 196608}},
        {"id": 10, "timestamp_ms": base + 10000, "event": {"type": "DownloadSucceeded", "nar_hash": demo_hash(), "store_path": "/gnu/store/1xq2v3demohello-hello-2.12.1", "size": 712704, "elapsed_ms": 1820}},
        {"id": 11, "timestamp_ms": base + 11000, "event": {"type": "SeedAdded", "nar_hash": demo_hash(), "store_path": "/gnu/store/1xq2v3demohello-hello-2.12.1", "nar_size": 712704}}
    ]))
}

async fn catalog() -> Json<Value> {
    Json(json!([
        {
            "hash_part": "1xq2v3demohello",
            "store_path": "/gnu/store/1xq2v3demohello-hello-2.12.1",
            "nar_size": 712704,
            "nar_hash": demo_hash(),
            "p2p_available": true
        },
        {
            "hash_part": "8mc4v3demoguile",
            "store_path": "/gnu/store/8mc4v3demoguile-guile-3.0.9",
            "nar_size": 14860288,
            "nar_hash": "sha256:guile-demo-nar-hash",
            "p2p_available": false
        }
    ]))
}

async fn seeds() -> Json<Value> {
    Json(json!([
        {
            "nar_hash": demo_hash(),
            "nar_size": 712704,
            "block_count": 11,
            "block_size": 65536,
            "store_path": "/gnu/store/1xq2v3demohello-hello-2.12.1",
            "source": "manual"
        },
        {
            "nar_hash": "sha256:profile-demo-seed",
            "nar_size": 221184,
            "block_count": 4,
            "block_size": 65536,
            "store_path": "/gnu/store/9zzzdemo-profile-hook-0.1",
            "source": "cache"
        },
        {
            "nar_hash": "sha256:demo-downloaded-seed-without-store-path",
            "nar_size": 98304,
            "block_count": 2,
            "block_size": 65536,
            "source": "downloaded"
        }
    ]))
}

async fn seed() -> (StatusCode, Json<Value>) {
    (
        StatusCode::ACCEPTED,
        Json(json!({
            "nar_hash": "sha256:demo-manual-seed",
            "store_path": "/gnu/store/demo-manual-package-1.0",
            "nar_size": 131072
        })),
    )
}

async fn seed_delete(Path(_hash): Path<String>) -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn packages() -> Json<Value> {
    Json(json!([
        {
            "source": "system",
            "name": "hello",
            "version": "2.12.1",
            "output": "out",
            "store_path": "/gnu/store/1xq2v3demohello-hello-2.12.1",
            "seeded": true
        },
        {
            "source": "system",
            "name": "guile",
            "version": "3.0.9",
            "output": "out",
            "store_path": "/gnu/store/8mc4v3demoguile-guile-3.0.9",
            "seeded": false
        },
        {
            "source": "home",
            "name": "ripgrep",
            "version": "14.1.1",
            "output": "out",
            "store_path": "/gnu/store/7rgdemo-ripgrep-14.1.1",
            "seeded": false
        }
    ]))
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_ws)
}

async fn handle_ws(mut socket: WebSocket) {
    let events = [
        json!({"type": "PeerDialStarted", "peer_id": "12D3KooWLivePeer"}),
        json!({"type": "PeerDialFailed", "peer_id": "12D3KooWLivePeer", "reason": "demo timeout while dialing"}),
        json!({"type": "BlockReceived", "nar_hash": demo_hash(), "peer_id": "12D3KooWCharlesSeedNode", "indices": [9, 10], "bytes": 122880}),
        json!({"type": "BlockServed", "nar_hash": demo_hash(), "peer_id": "12D3KooWCarolFetchNode", "indices": [0, 1]}),
        json!({"type": "CatalogEntry", "hash_part": "5demoliveevent", "store_path": "/gnu/store/5demoliveevent-demo-live-1.0", "nar_size": 98304, "nar_hash": "sha256:demo-live-event", "p2p_available": true}),
    ];

    for event in events {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if socket.send(Message::Text(event.to_string())).await.is_err() {
            break;
        }
    }
}

fn demo_hash() -> &'static str {
    "sha256:demo-hello-nar-hash"
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::router;

    #[tokio::test]
    async fn demo_status_endpoint_marks_demo_mode() {
        let response = router()
            .oneshot(Request::builder().uri("/api/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["demo"], true);
        assert_eq!(body["connected_peers"], 3);
        assert_eq!(body["connectivity"]["state"], "shareable");
    }

    #[tokio::test]
    async fn demo_dashboard_serves_embedded_html() {
        let response = router()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("guix-p2p dashboard"));
        assert!(body.contains("open evidence"));
        assert!(body.contains("stop seeding this nar"));
        assert!(body.contains("metadata pending"));
        assert!(body.contains("aria-label"));
    }

    #[tokio::test]
    async fn demo_seeds_include_hash_only_downloaded_seed() {
        let response = router()
            .oneshot(Request::builder().uri("/api/seeds").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            body.as_array()
                .unwrap()
                .iter()
                .any(|seed| { seed["source"] == "downloaded" && seed.get("store_path").is_none() })
        );
    }

    #[tokio::test]
    async fn demo_share_info_endpoint_matches_dashboard_button() {
        let response = router()
            .oneshot(Request::builder().uri("/api/share-info").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body["config_snippet"].as_str().unwrap().contains("bootstrap_peers ="));
    }

    #[tokio::test]
    async fn demo_diagnostics_endpoint_supports_net_detail() {
        let response = router()
            .oneshot(Request::builder().uri("/api/diagnostics").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["connectivity"]["state"], "shareable");
        assert!(
            body["checks"].as_array().unwrap().iter().any(|check| check["id"] == "nat-address")
        );
    }
}
