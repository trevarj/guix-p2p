use std::{
    collections::{HashMap, HashSet},
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use libp2p::PeerId;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    channel::{NotifyRx, NotifyTx, SwarmCommand, SwarmNotification},
    config::{Config, SubstitutePolicy},
    connection::ConnectionManager,
    dashboard::{self, BuildRegistry, DashboardEvent, ObservedBuild},
    dht::ProviderCache,
    http_client::HttpClientError,
    nar_hash,
    nar_restore::restore_nar_to_destination,
    nar_store::NarStore,
    narinfo::NarinfoCache,
    reputation::ReputationTracker,
    store_path,
    swarm::{
        block::BlockInfo,
        codec::{BlockData, BlockRequest, BlockResponse},
        downloader::{ActiveDownload, DownloadError},
    },
};

mod protocol;

pub use protocol::{
    DaemonCommand, LineBuffer, ReplyWriter, format_trace_progress, format_trace_started,
    format_trace_succeeded, parse_command_line, read_command,
};

pub fn extract_hash_part(store_path: &str) -> Result<String, String> {
    store_path::hash_part(store_path)
}

/// Extract the raw SHA-256 bytes from a narinfo NarHash field.
fn extract_nar_hash_bytes(nar_hash: &str) -> Option<[u8; 32]> {
    nar_hash::sha256_bytes(nar_hash)
}

pub async fn run_query_mode(
    cache: &ProviderCache,
    query_tx: &UnboundedSender<String>,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    mut notify_rx: NotifyRx,
    narinfo_cache: &Arc<Mutex<NarinfoCache>>,
    config: &Config,
    client: &reqwest::Client,
) -> anyhow::Result<()> {
    let _ = cmd_tx;
    let mut reply = ReplyWriter::new();

    loop {
        let cmd = match read_command() {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                tracing::error!("Failed to read daemon command: {}", e);
                break;
            },
        };

        match cmd {
            DaemonCommand::Have(paths) => {
                handle_have(
                    cache,
                    query_tx,
                    &mut reply,
                    &paths,
                    config,
                    &mut notify_rx,
                    narinfo_cache,
                    client,
                    None,
                )
                .await;
            },
            DaemonCommand::Info(paths) => {
                handle_info(
                    config,
                    query_tx,
                    &mut notify_rx,
                    narinfo_cache,
                    &mut reply,
                    &paths,
                    client,
                    None,
                )
                .await;
            },
            DaemonCommand::Substitute { .. } => {},
        }
        // Drain any pending broadcast notifications to prevent lagging
        while notify_rx.try_recv().is_ok() {}
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_have(
    cache: &ProviderCache,
    query_tx: &UnboundedSender<String>,
    reply: &mut ReplyWriter,
    paths: &[String],
    config: &Config,
    notify_rx: &mut NotifyRx,
    narinfo_cache: &Arc<Mutex<NarinfoCache>>,
    client: &reqwest::Client,
    event_tx: Option<&dashboard::EventBus>,
) {
    for path in paths {
        let hash_part = match extract_hash_part(path) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("Skipping: {} ({})", path, e);
                continue;
            },
        };

        tracing::debug!("have query: {} (hash_part={})", path, hash_part);

        match config.substitute_policy {
            SubstitutePolicy::HttpFirst | SubstitutePolicy::P2pFirst => {
                // Fallback policies can use P2P or HTTP, but we still need
                // usable narinfo before advertising a path to Guix.
                let cached_info = narinfo_cache.lock().unwrap().get(&hash_part);
                let info_result = match (cached_info, config.local_narinfo_path.is_some()) {
                    (Some(info), _) => Ok(info),
                    (None, true) => {
                        tracing::info!(
                            "have: skipping {} (policy={:?}, not in local narinfo metadata)",
                            path,
                            config.substitute_policy
                        );
                        continue;
                    },
                    (None, false) => {
                        crate::http_client::fetch_narinfo(config, &hash_part, narinfo_cache, client)
                            .await
                    },
                };
                match info_result {
                    Ok(info) => {
                        tracing::info!(
                            "have: claiming {} (policy={:?}, narinfo available)",
                            path,
                            config.substitute_policy
                        );
                        if let Err(e) = reply.write_line(path) {
                            tracing::error!("have reply for {}: {}", path, e);
                        }
                        if let Some(tx) = event_tx {
                            let nar_hash = extract_nar_hash_bytes(&info.nar_hash).map(hex::encode);
                            let provider_key = nar_hash.as_deref().unwrap_or(&hash_part);
                            let p2p_available =
                                crate::dht::has_providers(cache, provider_key).await;
                            let _ = tx.send(DashboardEvent::CatalogEntry {
                                hash_part: hash_part.clone(),
                                store_path: Some(info.store_path),
                                nar_size: Some(info.nar_size),
                                nar_hash,
                                p2p_available,
                            });
                        }
                    },
                    Err(e) => {
                        tracing::info!(
                            "have: skipping {} (policy={:?}, narinfo unavailable: {})",
                            path,
                            config.substitute_policy,
                            e
                        );
                    },
                }
            },
            SubstitutePolicy::P2pOnly => {
                let narinfo = match crate::http_client::fetch_narinfo(
                    config,
                    &hash_part,
                    narinfo_cache,
                    client,
                )
                .await
                {
                    Ok(info) => info,
                    Err(e) => {
                        tracing::info!(
                            "have: skipping {} (p2p-only, narinfo unavailable: {})",
                            path,
                            e
                        );
                        continue;
                    },
                };

                let nar_hash_bytes = match extract_nar_hash_bytes(&narinfo.nar_hash) {
                    Some(bytes) => bytes,
                    None => {
                        tracing::warn!("have: skipping {} (invalid nar hash)", path);
                        continue;
                    },
                };
                let dht_key = hex::encode(nar_hash_bytes);
                let _ = query_tx.send(dht_key.clone());
                let query_timeout =
                    tokio::time::Duration::from_secs(config.request_timeout_secs.min(5));
                let providers =
                    wait_for_providers_for_duration(notify_rx, &dht_key, query_timeout).await;
                let p2p_available = providers.len() >= config.min_providers;

                if p2p_available {
                    tracing::info!(
                        "have: claiming {} (p2p-only, {}/{}) providers found",
                        path,
                        providers.len(),
                        config.min_providers
                    );
                    if let Err(e) = reply.write_line(path) {
                        tracing::error!("have reply for {}: {}", path, e);
                    }
                } else {
                    tracing::info!(
                        "have: skipping {} (p2p-only, only {}/{}) providers found",
                        path,
                        providers.len(),
                        config.min_providers
                    );
                }

                if let Some(tx) = event_tx {
                    let _ = tx.send(DashboardEvent::CatalogEntry {
                        hash_part: hash_part.clone(),
                        store_path: Some(narinfo.store_path.clone()),
                        nar_size: Some(narinfo.nar_size),
                        nar_hash: Some(dht_key.clone()),
                        p2p_available,
                    });
                }
            },
        }
    }

    if let Err(e) = reply.write_end() {
        tracing::error!("have end marker: {}", e);
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_info(
    config: &Config,
    _query_tx: &UnboundedSender<String>,
    _notify_rx: &mut NotifyRx,
    cache: &Mutex<NarinfoCache>,
    reply: &mut ReplyWriter,
    paths: &[String],
    client: &reqwest::Client,
    event_tx: Option<&dashboard::EventBus>,
) {
    for path in paths {
        let hash_part = match extract_hash_part(path) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("Cannot extract hash from {}: {}", path, e);
                continue;
            },
        };

        let cached_info = cache.lock().unwrap().get(&hash_part);
        let info_result = match (config.substitute_policy, cached_info) {
            (SubstitutePolicy::P2pOnly, Some(info)) => Ok(info),
            (SubstitutePolicy::P2pOnly, None) => {
                tracing::debug!("No cached p2p-only narinfo for {}", hash_part);
                continue;
            },
            (_, Some(info)) => Ok(info),
            (_, None) if config.local_narinfo_path.is_some() => {
                tracing::debug!("No local narinfo for {}", hash_part);
                continue;
            },
            (_, None) => crate::http_client::fetch_narinfo(config, &hash_part, cache, client).await,
        };

        match info_result {
            Ok(info) => {
                let nar_hash_bytes = match extract_nar_hash_bytes(&info.nar_hash) {
                    Some(bytes) => bytes,
                    None => {
                        tracing::warn!("info: skipping {} (invalid nar hash)", path);
                        continue;
                    },
                };
                let dht_key = hex::encode(nar_hash_bytes);

                if let Some(tx) = event_tx {
                    let _ = tx.send(DashboardEvent::CatalogEntry {
                        hash_part: hash_part.clone(),
                        store_path: Some(info.store_path.clone()),
                        nar_size: Some(info.nar_size),
                        nar_hash: Some(dht_key),
                        p2p_available: false,
                    });
                }
                let _ = reply.write_line(&info.store_path);
                let deriver = info.deriver.as_deref().map(guix_store_path).unwrap_or_default();
                let _ = reply.write_line(&deriver);
                let _ = reply.write_line(&info.references.len().to_string());
                for r in &info.references {
                    let _ = reply.write_line(&guix_store_path(r));
                }
                let download_size = info.urls.first().map(|u| u.file_size).unwrap_or(0);
                let _ = reply.write_line(&download_size.to_string());
                let _ = reply.write_line(&info.nar_size.to_string());
            },
            Err(e) => {
                tracing::debug!("No narinfo for {}: {}", hash_part, e);
            },
        }
    }

    let _ = reply.write_end();
}

fn guix_store_path(path_or_basename: &str) -> String {
    store_path::from_narinfo_path(path_or_basename)
}

#[allow(clippy::too_many_arguments)]
pub async fn run_substitute_mode(
    cache: &ProviderCache,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    mut notify_rx: NotifyRx,
    narinfo_cache: &Arc<Mutex<NarinfoCache>>,
    config: &Config,
    _query_tx: &UnboundedSender<String>,
    reputation: &Arc<Mutex<ReputationTracker>>,
    conn_mgr: &Arc<Mutex<ConnectionManager>>,
    client: &reqwest::Client,
    nar_store: &Arc<Mutex<NarStore>>,
) -> anyhow::Result<()> {
    let mut reply = ReplyWriter::new();

    loop {
        let cmd = match read_command() {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                tracing::error!("Failed to read daemon command: {}", e);
                break;
            },
        };

        if let DaemonCommand::Substitute { path, dest } = cmd {
            let dummy_registry: BuildRegistry =
                Arc::new(Mutex::new(std::collections::HashMap::new()));
            let (dummy_tx, _) = tokio::sync::broadcast::channel(1);
            try_swarm_substitute(
                config,
                cache,
                cmd_tx,
                &mut reply,
                &path,
                &dest,
                &mut notify_rx,
                narinfo_cache,
                reputation,
                conn_mgr,
                &dummy_registry,
                &dummy_tx,
                client,
                nar_store,
            )
            .await;
        }
    }

    Ok(())
}

/// Full substitute download according to the configured policy.
/// - P2pOnly: swarm only, return not-found on failure
/// - P2pFirst: try swarm, fall back to HTTP nar download
/// - HttpFirst: try HTTP nar download, fall back to swarm
#[allow(clippy::too_many_arguments)]
async fn try_swarm_substitute(
    config: &Config,
    cache: &ProviderCache,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    reply: &mut ReplyWriter,
    path: &str,
    dest: &str,
    notify_rx: &mut NotifyRx,
    narinfo_cache: &Arc<Mutex<NarinfoCache>>,
    reputation: &Arc<Mutex<ReputationTracker>>,
    conn_mgr: &Arc<Mutex<ConnectionManager>>,
    build_registry: &BuildRegistry,
    event_tx: &dashboard::EventBus,
    client: &reqwest::Client,
    nar_store: &Arc<Mutex<NarStore>>,
) {
    let hash_part = match extract_hash_part(path) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Cannot extract hash from {}: {}", path, e);
            let _ = reply.write_line("not-found");
            return;
        },
    };

    let cached_info = narinfo_cache.lock().unwrap().get(&hash_part);
    let narinfo = match (cached_info, config.local_narinfo_path.is_some()) {
        (Some(info), _) => info,
        (None, true) => {
            tracing::warn!("No local narinfo for {}", hash_part);
            let _ = reply.write_line("not-found");
            return;
        },
        (None, false) => {
            match crate::http_client::fetch_narinfo(config, &hash_part, narinfo_cache, client).await
            {
                Ok(info) => info,
                Err(e) => {
                    tracing::warn!("Cannot fetch narinfo for {}: {}", hash_part, e);
                    let _ = reply.write_line("not-found");
                    return;
                },
            }
        },
    };

    let store_path = narinfo.store_path.clone();
    let nar_size = narinfo.nar_size;
    let nar_hash = narinfo.nar_hash.clone();
    let nar_hash_bytes = match extract_nar_hash_bytes(&nar_hash) {
        Some(bytes) => bytes,
        None => {
            tracing::warn!("Cannot decode nar hash for {}: {}", hash_part, nar_hash);
            let _ = reply.write_line("not-found");
            return;
        },
    };
    let nar_hash_hex = hex::encode(nar_hash_bytes);

    tracing::info!(
        policy = %config.substitute_policy,
        store = %store_path,
        "handling substitute request"
    );

    // Populate build registry from observed narinfo
    {
        let mut reg = build_registry.lock().unwrap();
        reg.entry(hash_part.clone())
            .and_modify(|b: &mut ObservedBuild| {
                b.store_path = Some(narinfo.store_path.clone());
                b.nar_size = Some(narinfo.nar_size);
                b.references = narinfo.references.clone();
                b.deriver = narinfo.deriver.clone();
                b.narinfo_raw = Some(narinfo.signed_portion.clone());
            })
            .or_insert_with(|| ObservedBuild {
                nar_hash: nar_hash_hex.clone(),
                store_path: Some(narinfo.store_path.clone()),
                nar_size: Some(narinfo.nar_size),
                references: narinfo.references.clone(),
                deriver: narinfo.deriver.clone(),
                narinfo_raw: Some(narinfo.signed_portion.clone()),
                providers: vec![],
                downloaded_at: None,
                download_size: None,
            });
    }

    let _ = event_tx.send(DashboardEvent::BuildDiscovered {
        nar_hash: nar_hash_hex.clone(),
        store_path: Some(store_path.clone()),
        nar_size: Some(nar_size),
    });

    let _ = event_tx.send(DashboardEvent::CatalogEntry {
        hash_part: hash_part.clone(),
        store_path: Some(store_path.clone()),
        nar_size: Some(nar_size),
        nar_hash: Some(nar_hash_hex.clone()),
        p2p_available: false,
    });

    let result = match config.substitute_policy {
        SubstitutePolicy::P2pOnly => {
            tracing::info!("p2p-only policy active; HTTP nar fallback disabled");
            let _ = reply.write_trace(&format_trace_started(
                &store_path,
                &format!("p2p://{}", hash_part),
                nar_size,
            ));
            try_p2p_download(
                config,
                cache,
                cmd_tx,
                notify_rx,
                &nar_hash_hex,
                &nar_hash_bytes,
                &store_path,
                nar_size,
                &hash_part,
                reputation,
                conn_mgr,
                event_tx,
                client,
                narinfo_cache,
            )
            .await
        },
        SubstitutePolicy::P2pFirst => {
            let _ = reply.write_trace(&format_trace_started(
                &store_path,
                &format!("p2p://{}", hash_part),
                nar_size,
            ));
            match try_p2p_download(
                config,
                cache,
                cmd_tx,
                notify_rx,
                &nar_hash_hex,
                &nar_hash_bytes,
                &store_path,
                nar_size,
                &hash_part,
                reputation,
                conn_mgr,
                event_tx,
                client,
                narinfo_cache,
            )
            .await
            {
                Ok(nar_data) => Ok(nar_data),
                Err(_) => {
                    tracing::info!(
                        hash = %nar_hash_hex,
                        "P2P failed, falling back to HTTP"
                    );
                    let _ = reply.write_trace(&format_trace_started(
                        &store_path,
                        "https://fallback",
                        nar_size,
                    ));
                    try_http_download(config, &narinfo, client, event_tx, &store_path).await
                },
            }
        },
        SubstitutePolicy::HttpFirst => {
            let _ =
                reply.write_trace(&format_trace_started(&store_path, "https://fallback", nar_size));
            match try_http_download(config, &narinfo, client, event_tx, &store_path).await {
                Ok(nar_data) => Ok(nar_data),
                Err(e) => {
                    tracing::info!(
                        hash = %nar_hash_hex,
                        error = %e,
                        "HTTP failed, falling back to P2P"
                    );
                    let _ = reply.write_trace(&format_trace_started(
                        &store_path,
                        &format!("p2p://{}", hash_part),
                        nar_size,
                    ));
                    try_p2p_download(
                        config,
                        cache,
                        cmd_tx,
                        notify_rx,
                        &nar_hash_hex,
                        &nar_hash_bytes,
                        &store_path,
                        nar_size,
                        &hash_part,
                        reputation,
                        conn_mgr,
                        event_tx,
                        client,
                        narinfo_cache,
                    )
                    .await
                },
            }
        },
    };

    match result {
        Ok(nar_data) => {
            let dest_path = PathBuf::from(dest);
            let size = nar_data.len() as u64;

            // Verify nar hash against narinfo's expected hash
            let hash = Sha256::digest(&nar_data);
            let actual_nar_hash = hex::encode(hash);
            let expected_nar_hash = nar_hash_hex.as_str();

            if !expected_nar_hash.eq_ignore_ascii_case(&actual_nar_hash) {
                tracing::error!(
                    "Nar hash mismatch for {}: expected {}, got {}",
                    store_path,
                    expected_nar_hash,
                    actual_nar_hash
                );

                // Delete the corrupted file
                let _ = tokio::fs::remove_file(&dest_path).await;

                let _ = event_tx.send(DashboardEvent::DownloadFailed {
                    nar_hash: nar_hash_hex.clone(),
                    store_path: store_path.clone(),
                    reason: format!(
                        "hash mismatch: expected sha256:{}, got sha256:{}",
                        expected_nar_hash, actual_nar_hash
                    ),
                });

                let _ = reply.write_line(&format!(
                    "hash-mismatch sha256 sha256:{} sha256:{}",
                    expected_nar_hash, actual_nar_hash
                ));
                return;
            }

            if reply.is_socket() {
                if let Err(e) = reply.write_nar_data(&nar_data) {
                    tracing::error!("Failed to queue nar for socket relay: {}", e);
                    let _ = reply.write_line("not-found");
                    let _ = event_tx.send(DashboardEvent::DownloadFailed {
                        nar_hash: nar_hash_hex.clone(),
                        store_path: store_path.clone(),
                        reason: format!("socket relay write error: {}", e),
                    });
                    return;
                }
            } else {
                // Direct substitute mode runs as guix-daemon's child and can
                // restore the destination path itself.
                let temp_path = std::env::temp_dir().join(format!(
                    "guix-p2p-direct-{}-{}.nar",
                    std::process::id(),
                    expected_nar_hash
                ));
                if let Err(e) = tokio::fs::write(&temp_path, &nar_data).await {
                    tracing::error!("Failed to write temporary nar {}: {}", temp_path.display(), e);
                    let _ = reply.write_line("not-found");
                    let _ = event_tx.send(DashboardEvent::DownloadFailed {
                        nar_hash: nar_hash_hex.clone(),
                        store_path: store_path.clone(),
                        reason: format!("temporary nar write error: {}", e),
                    });
                    return;
                }
                if let Err(e) = restore_nar_to_destination(&temp_path, &dest_path).await {
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    tracing::error!("Failed to restore nar to {}: {}", dest_path.display(), e);
                    let _ = reply.write_line("not-found");
                    let _ = event_tx.send(DashboardEvent::DownloadFailed {
                        nar_hash: nar_hash_hex.clone(),
                        store_path: store_path.clone(),
                        reason: format!("restore error: {}", e),
                    });
                    return;
                }
                let _ = tokio::fs::remove_file(&temp_path).await;
            }

            // Save nar to local store for re-seeding
            {
                let mut store = nar_store.lock().unwrap();
                if store.has_nar(&nar_hash_hex) {
                    tracing::debug!("nar already in store, skipping save");
                } else if let Err(e) =
                    store.save_with_store_path(&nar_hash_hex, &nar_data, Some(store_path.clone()))
                {
                    tracing::warn!("failed to save nar to store for re-seeding: {}", e);
                } else {
                    drop(store);
                    let _ =
                        cmd_tx.send(SwarmCommand::StartProviding { hash: nar_hash_hex.clone() });
                    let _ = event_tx.send(DashboardEvent::SeedAdded {
                        nar_hash: nar_hash_hex.clone(),
                        store_path: Some(store_path.clone()),
                        nar_size: size,
                    });
                }
            }

            // Mark build as downloaded in registry
            {
                let mut reg = build_registry.lock().unwrap();
                if let Some(b) = reg.get_mut(hash_part.as_str()) {
                    b.downloaded_at = Some(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                    );
                    b.download_size = Some(size);
                }
            }

            let _ = event_tx.send(DashboardEvent::DownloadSucceeded {
                nar_hash: nar_hash_hex.clone(),
                store_path: store_path.clone(),
                size,
                elapsed_ms: 0,
            });

            let _ = reply.write_trace(&format_trace_succeeded(
                &store_path,
                &format!("p2p://{}", hash_part),
                size,
            ));

            let _ = reply.write_line(&format!("success sha256:{} {}", nar_hash_hex, size));
            tracing::info!("Substitute download succeeded for {}", store_path);
        },
        Err(reason) => {
            tracing::error!("Substitute download failed for {}: {}", store_path, reason);

            let _ = event_tx.send(DashboardEvent::DownloadFailed {
                nar_hash: nar_hash_hex.clone(),
                store_path: store_path.clone(),
                reason: reason.to_string(),
            });

            let _ = reply.write_line("not-found");
        },
    }
}

/// Attempt P2P swarm download.
#[allow(clippy::too_many_arguments)]
async fn try_p2p_download(
    config: &Config,
    cache: &ProviderCache,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    notify_rx: &mut NotifyRx,
    nar_hash: &str,
    nar_hash_bytes: &[u8; 32],
    store_path: &str,
    nar_size: u64,
    _hash_part: &str,
    reputation: &Arc<Mutex<ReputationTracker>>,
    conn_mgr: &Arc<Mutex<ConnectionManager>>,
    event_tx: &dashboard::EventBus,
    _client: &reqwest::Client,
    _narinfo_cache: &Arc<Mutex<NarinfoCache>>,
) -> Result<Vec<u8>, String> {
    tracing::info!(hash = %nar_hash, size = nar_size, "Attempting P2P download");

    let _ = event_tx.send(DashboardEvent::DownloadStarted {
        nar_hash: nar_hash.to_string(),
        store_path: store_path.to_string(),
        nar_size,
    });

    let dht_key = hex::encode(nar_hash_bytes);
    let _ = cmd_tx.send(SwarmCommand::GetProviders { hash: dht_key.clone() });

    let providers = wait_for_providers(cache, notify_rx, &dht_key, config).await;

    if providers.len() < config.min_providers {
        return Err(format!(
            "not enough P2P providers ({}/{})",
            providers.len(),
            config.min_providers
        ));
    }

    tracing::info!("Found {} P2P providers for {}", providers.len(), nar_hash);

    let mut providers =
        select_provider_candidates(&providers, reputation, conn_mgr, config.max_peers_per_download);

    if providers.len() < config.min_providers {
        tracing::debug!(
            "Only {} usable cached P2P providers for {}; waiting for fresh DHT results",
            providers.len(),
            nar_hash
        );
        let refreshed = wait_for_providers_for_duration(
            notify_rx,
            &dht_key,
            tokio::time::Duration::from_secs(config.request_timeout_secs),
        )
        .await;
        let merged = merge_provider_lists(&providers, &refreshed);
        providers = select_provider_candidates(
            &merged,
            reputation,
            conn_mgr,
            config.max_peers_per_download,
        );

        if providers.len() < config.min_providers {
            return Err(format!(
                "not enough usable P2P providers after reputation/backoff filtering ({}/{})",
                providers.len(),
                config.min_providers
            ));
        }
    }

    let handshakes = handshake_with_providers(
        cmd_tx,
        notify_rx,
        &providers,
        *nar_hash_bytes,
        config.max_peers_per_download,
        reputation,
        conn_mgr,
    )
    .await;

    if handshakes.is_empty() {
        return Err("no successful P2P handshakes".into());
    }
    if handshakes.len() < config.min_providers {
        return Err(format!(
            "not enough successful P2P handshakes ({}/{})",
            handshakes.len(),
            config.min_providers
        ));
    }

    let download_block_info = if nar_size == 0 {
        let first = handshakes.first().ok_or_else(|| "no successful P2P handshakes".to_string())?;
        BlockInfo {
            block_count: first.block_count,
            block_size: config.block_size,
            last_block_size: config.block_size,
            block_hashes: first.block_hashes.clone(),
        }
    } else {
        let mut info = BlockInfo::from_file_size(nar_size, config.block_size);
        if let Some(first) = handshakes.first() {
            info.set_hashes(first.block_hashes.clone());
        }
        info
    };

    let download_start = std::time::Instant::now();

    let block_download = BlockDownloadContext { cmd_tx, notify_rx, config, event_tx };

    match download_blocks_from_peers(
        block_download,
        &handshakes,
        nar_size,
        download_block_info,
        nar_hash,
    )
    .await
    {
        Ok((nar_data, _verified_hash)) => {
            let elapsed_ms = download_start.elapsed().as_millis() as u64;

            let _ = event_tx.send(DashboardEvent::DownloadSucceeded {
                nar_hash: nar_hash.to_string(),
                store_path: store_path.to_string(),
                size: nar_data.len() as u64,
                elapsed_ms,
            });

            Ok(nar_data)
        },
        Err(e) => Err(format!("P2P swarm download failed: {}", e)),
    }
}

/// Attempt HTTP nar download from substitute servers.
async fn try_http_download(
    config: &Config,
    narinfo: &crate::narinfo::Narinfo,
    client: &reqwest::Client,
    _event_tx: &dashboard::EventBus,
    store_path: &str,
) -> Result<Vec<u8>, String> {
    tracing::info!(store = %store_path, "Attempting HTTP nar download");

    match crate::http_client::download_nar_http(config, narinfo, client).await {
        Ok(nar_data) => {
            tracing::info!(
                "HTTP nar download succeeded for {} ({} bytes)",
                store_path,
                nar_data.len()
            );
            Ok(nar_data)
        },
        Err(HttpClientError::NotFound) => Err("HTTP nar not found on any substitute server".into()),
        Err(HttpClientError::BadSignature) => Err("narinfo signature verification failed".into()),
        Err(e) => Err(format!("HTTP nar download failed: {}", e)),
    }
}

/// Wait for provider notifications for the given DHT key, with a timeout.
async fn wait_for_providers(
    cache: &ProviderCache,
    notify_rx: &mut NotifyRx,
    dht_key: &str,
    config: &Config,
) -> Vec<PeerId> {
    let cached = crate::dht::get_providers(cache, dht_key).await;
    if cached.len() >= config.min_providers {
        tracing::debug!("Using {} cached providers for {}", cached.len(), dht_key);
        return cached;
    }

    wait_for_providers_for_duration(
        notify_rx,
        dht_key,
        tokio::time::Duration::from_secs(config.request_timeout_secs),
    )
    .await
}

async fn wait_for_providers_for_duration(
    notify_rx: &mut NotifyRx,
    dht_key: &str,
    timeout: tokio::time::Duration,
) -> Vec<PeerId> {
    let deadline = tokio::time::Instant::now() + timeout;
    let collect_grace = tokio::time::Duration::from_secs(3);
    let mut collect_until = deadline;
    let mut providers = Vec::new();
    let mut seen = HashSet::new();

    loop {
        let remaining = collect_until.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            if providers.is_empty() {
                tracing::debug!("Provider lookup timed out for {}", dht_key);
            }
            return providers;
        }

        let wait_for = remaining.min(tokio::time::Duration::from_secs(1));
        match tokio::time::timeout(wait_for, notify_rx.recv()).await {
            Ok(Ok(SwarmNotification::ProvidersFound { hash, peers })) => {
                if hash == dht_key {
                    let new_providers: Vec<_> =
                        peers.into_iter().filter(|peer| seen.insert(*peer)).collect();
                    let added = new_providers.len();
                    providers.extend(new_providers);

                    if added > 0 && collect_until == deadline {
                        collect_until = (tokio::time::Instant::now() + collect_grace).min(deadline);
                    }
                }
            },
            Ok(Ok(_)) => {},
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                tracing::warn!("Notification receiver lagged by {} messages", n);
                continue;
            },
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                return Vec::new();
            },
            Err(_elapsed) => continue,
        }
    }
}

fn merge_provider_lists(left: &[PeerId], right: &[PeerId]) -> Vec<PeerId> {
    let mut seen = HashSet::new();
    left.iter().chain(right.iter()).copied().filter(|peer| seen.insert(*peer)).collect()
}

fn select_provider_candidates(
    providers: &[PeerId],
    reputation: &Arc<Mutex<ReputationTracker>>,
    conn_mgr: &Arc<Mutex<ConnectionManager>>,
    max_peers: usize,
) -> Vec<PeerId> {
    let ranked = {
        let tracker = reputation.lock().unwrap();
        tracker.best_peers(providers, providers.len())
    };

    let mut selected = Vec::new();
    let mut skipped = 0;
    {
        let connections = conn_mgr.lock().unwrap();
        for peer in ranked {
            if selected.len() >= max_peers {
                break;
            }

            if connections.can_connect(&peer) {
                selected.push(peer);
            } else {
                skipped += 1;
            }
        }
    }

    if skipped > 0 {
        tracing::debug!(
            "Skipped {} provider candidates because connection backoff is active",
            skipped
        );
    }

    selected
}

/// Handshake results: for each peer, which blocks they have.
struct PeerHandshake {
    peer: PeerId,
    blocks_available: Vec<u32>,
    block_hashes: Vec<[u8; 32]>,
    block_count: u32,
}

struct BlockDownloadContext<'a> {
    cmd_tx: &'a UnboundedSender<SwarmCommand>,
    notify_rx: &'a mut NotifyRx,
    config: &'a Config,
    event_tx: &'a dashboard::EventBus,
}

#[derive(Clone, Debug)]
enum BlockFetchState {
    Pending,
    InFlight { peer: PeerId, requested_at: tokio::time::Instant },
    Complete,
}

#[derive(Debug)]
struct PeerFetchState {
    available: HashSet<u32>,
    in_flight: HashSet<u32>,
    failures: u32,
    bytes_received: u64,
}

/// Send Handshake requests and collect replies.
async fn handshake_with_providers(
    cmd_tx: &UnboundedSender<SwarmCommand>,
    notify_rx: &mut NotifyRx,
    providers: &[PeerId],
    nar_hash_bytes: [u8; 32],
    max_peers: usize,
    reputation: &Arc<Mutex<ReputationTracker>>,
    conn_mgr: &Arc<Mutex<ConnectionManager>>,
) -> Vec<PeerHandshake> {
    let max = providers.len().min(max_peers);
    let mut results = Vec::new();
    let mut pending = HashMap::new();

    // Send handshakes
    for peer in providers.iter().take(max) {
        conn_mgr.lock().unwrap().record_attempt(*peer);
        let request = BlockRequest::Handshake { nar_hash: nar_hash_bytes.to_vec() };
        let _ = cmd_tx.send(SwarmCommand::SendBlockRequest { peer: *peer, request });
        pending.insert(*peer, (1_usize, tokio::time::Instant::now()));
    }

    // Collect replies with timeout
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);

    while !pending.is_empty() && results.len() < max {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        match tokio::time::timeout(tokio::time::Duration::from_secs(1), notify_rx.recv()).await {
            Ok(Ok(SwarmNotification::BlockResponse {
                peer,
                response:
                    BlockResponse::HandshakeReply {
                        blocks_available, block_count, block_hashes, ..
                    },
            })) => {
                if !pending.contains_key(&peer) {
                    tracing::debug!("Ignoring unsolicited handshake reply from {}", peer);
                    continue;
                }

                tracing::info!(
                    "Handshake with {}: {} blocks available",
                    peer,
                    blocks_available.len()
                );

                let hashes: Vec<[u8; 32]> = block_hashes
                    .into_iter()
                    .map(|h| {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&h[..32.min(h.len())]);
                        arr
                    })
                    .collect();

                results.push(PeerHandshake {
                    peer,
                    blocks_available,
                    block_hashes: hashes,
                    block_count,
                });
                pending.remove(&peer);
            },
            Ok(Ok(_)) => {},
            Ok(Err(_)) => break,
            Err(_elapsed) => {
                let now = tokio::time::Instant::now();
                for (peer, (attempts, last_sent)) in &mut pending {
                    if *attempts < 3
                        && now.duration_since(*last_sent) >= tokio::time::Duration::from_secs(3)
                    {
                        let request = BlockRequest::Handshake { nar_hash: nar_hash_bytes.to_vec() };
                        let _ =
                            cmd_tx.send(SwarmCommand::SendBlockRequest { peer: *peer, request });
                        *attempts += 1;
                        *last_sent = now;
                        tracing::debug!(
                            "Retrying handshake with {} (attempt {}/{})",
                            peer,
                            attempts,
                            3
                        );
                    }
                }
                continue;
            },
        }
    }

    if !pending.is_empty() {
        let mut tracker = reputation.lock().unwrap();
        for peer in pending.keys() {
            tracing::warn!("Handshake with provider {} timed out", peer);
            tracker.record_failure(*peer);
        }
    }

    results
}

/// Orchestrate block downloads from peers.
async fn download_blocks_from_peers(
    ctx: BlockDownloadContext<'_>,
    handshakes: &[PeerHandshake],
    nar_size: u64,
    block_info: BlockInfo,
    nar_hash: &str,
) -> Result<(Vec<u8>, String), DownloadError> {
    let mut download = ActiveDownload::new(
        nar_hash.to_string(),
        nar_size,
        block_info.clone(),
        PathBuf::new(),
        String::new(),
    );

    for hs in handshakes {
        download.add_peer(hs.peer);
    }

    let total = block_info.block_count as usize;
    let mut block_states = vec![BlockFetchState::Pending; total];
    let mut peer_states: HashMap<PeerId, PeerFetchState> = handshakes
        .iter()
        .map(|hs| {
            let available =
                hs.blocks_available.iter().copied().filter(|idx| (*idx as usize) < total).collect();
            (
                hs.peer,
                PeerFetchState {
                    available,
                    in_flight: HashSet::new(),
                    failures: 0,
                    bytes_received: 0,
                },
            )
        })
        .collect();

    let overall_deadline = tokio::time::Instant::now()
        + tokio::time::Duration::from_secs(ctx.config.request_timeout_secs);
    let mut stall_deadline = tokio::time::Instant::now()
        + tokio::time::Duration::from_secs(ctx.config.stall_timeout_secs);
    let block_timeout = tokio::time::Duration::from_secs(ctx.config.stall_timeout_secs);
    let max_in_flight = ctx.config.max_in_flight_blocks_per_peer.max(1);

    dispatch_block_requests(
        ctx.cmd_tx,
        &mut peer_states,
        &mut block_states,
        nar_hash,
        max_in_flight,
    );

    loop {
        let remaining = overall_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            tracing::warn!("Block download timed out");
            break;
        }

        if tokio::time::Instant::now() > stall_deadline {
            tracing::warn!(
                "Block download stalled (no new blocks for {}s)",
                ctx.config.stall_timeout_secs
            );
            break;
        }

        let requeued = requeue_expired_blocks(&mut peer_states, &mut block_states, block_timeout);
        if requeued > 0 {
            tracing::debug!("Requeued {} stalled blocks", requeued);
        }

        dispatch_block_requests(
            ctx.cmd_tx,
            &mut peer_states,
            &mut block_states,
            nar_hash,
            max_in_flight,
        );

        let in_flight = peer_states.values().map(|peer| peer.in_flight.len()).sum::<usize>();
        if in_flight == 0
            && !block_states.iter().all(|state| matches!(state, BlockFetchState::Complete))
        {
            tracing::warn!("Block download has pending blocks with no available providers");
            break;
        }

        match tokio::time::timeout(tokio::time::Duration::from_secs(1), ctx.notify_rx.recv()).await
        {
            Ok(Ok(SwarmNotification::BlockResponse {
                peer,
                response: BlockResponse::Blocks { data },
            })) => {
                let blocks: Vec<(u32, Vec<u8>)> =
                    data.into_iter().map(|bd: BlockData| (bd.index, bd.data)).collect();
                let accepted = download.record_blocks(peer, &blocks);
                let newly_accepted = accepted.len();
                let accepted_set: HashSet<u32> = accepted.iter().copied().collect();

                if let Some(peer_state) = peer_states.get_mut(&peer) {
                    let bytes_received =
                        blocks.iter().map(|(_, data)| data.len() as u64).sum::<u64>();
                    peer_state.bytes_received += bytes_received;

                    for (idx, _) in &blocks {
                        peer_state.in_flight.remove(idx);
                    }
                }

                if newly_accepted > 0 {
                    let bytes = blocks
                        .iter()
                        .filter(|(idx, _)| accepted_set.contains(idx))
                        .map(|(_, data)| data.len() as u64)
                        .sum();
                    let _ = ctx.event_tx.send(DashboardEvent::BlockReceived {
                        nar_hash: nar_hash.to_string(),
                        peer_id: peer.to_string(),
                        indices: accepted.clone(),
                        bytes,
                    });
                }

                for idx in accepted {
                    if let Some(state) = block_states.get_mut(idx as usize)
                        && !matches!(state, BlockFetchState::Complete)
                    {
                        *state = BlockFetchState::Complete;
                    }
                }

                for (idx, _) in &blocks {
                    if !accepted_set.contains(idx)
                        && let Some(state) = block_states.get_mut(*idx as usize)
                    {
                        let should_retry = matches!(
                            state,
                            BlockFetchState::InFlight { peer: state_peer, .. } if *state_peer == peer
                        );
                        if should_retry {
                            *state = BlockFetchState::Pending;
                        }
                    }
                }

                if newly_accepted > 0 {
                    stall_deadline = tokio::time::Instant::now()
                        + tokio::time::Duration::from_secs(ctx.config.stall_timeout_secs);
                }

                if download.is_complete() {
                    break;
                }
            },
            Ok(Ok(_)) => {},
            Ok(Err(_)) => break,
            Err(_elapsed) => continue,
        }
    }

    if !download.is_complete() {
        return Err(DownloadError::Incomplete);
    }

    let nar = download.assemble()?;

    let hash = Sha256::digest(&nar);
    let hash_hex = format!("sha256:{:x}", hash);

    Ok((nar, hash_hex))
}

fn requeue_expired_blocks(
    peer_states: &mut HashMap<PeerId, PeerFetchState>,
    block_states: &mut [BlockFetchState],
    block_timeout: tokio::time::Duration,
) -> usize {
    let now = tokio::time::Instant::now();
    let mut requeued = 0;

    for (idx, state) in block_states.iter_mut().enumerate() {
        let expired_peer = match state {
            BlockFetchState::InFlight { peer, requested_at }
                if now.duration_since(*requested_at) >= block_timeout =>
            {
                Some(*peer)
            },
            _ => None,
        };

        if let Some(peer) = expired_peer {
            if let Some(peer_state) = peer_states.get_mut(&peer) {
                peer_state.in_flight.remove(&(idx as u32));
                peer_state.failures += 1;
            }
            *state = BlockFetchState::Pending;
            requeued += 1;
        }
    }

    requeued
}

fn dispatch_block_requests(
    cmd_tx: &UnboundedSender<SwarmCommand>,
    peer_states: &mut HashMap<PeerId, PeerFetchState>,
    block_states: &mut [BlockFetchState],
    nar_hash: &str,
    max_in_flight: usize,
) -> usize {
    let mut batches: HashMap<PeerId, Vec<u32>> = HashMap::new();
    let now = tokio::time::Instant::now();

    for (idx, state) in block_states.iter_mut().enumerate() {
        if !matches!(state, BlockFetchState::Pending) {
            continue;
        }

        let block_idx = idx as u32;
        let peer = peer_states
            .iter()
            .filter(|(_, peer_state)| {
                peer_state.in_flight.len() < max_in_flight
                    && peer_state.available.contains(&block_idx)
            })
            .min_by_key(|(_, peer_state)| (peer_state.in_flight.len(), peer_state.failures))
            .map(|(peer, _)| *peer);

        if let Some(peer) = peer {
            if let Some(peer_state) = peer_states.get_mut(&peer) {
                peer_state.in_flight.insert(block_idx);
            }
            *state = BlockFetchState::InFlight { peer, requested_at: now };
            batches.entry(peer).or_default().push(block_idx);
        }
    }

    let dispatched = batches.values().map(Vec::len).sum();
    for (peer, indices) in batches {
        let n = indices.len();
        let request = BlockRequest::GetBlocks { nar_hash: nar_hash_bytes(nar_hash), indices };
        tracing::debug!("Requested {} blocks from {}", n, peer);
        let _ = cmd_tx.send(SwarmCommand::SendBlockRequest { peer, request });
    }

    dispatched
}

fn nar_hash_bytes(nar_hash: &str) -> Vec<u8> {
    nar_hash::sha256_vec(nar_hash)
}

#[allow(clippy::too_many_arguments)]
pub async fn run_daemon_mode(
    cache: &ProviderCache,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    query_tx: &UnboundedSender<String>,
    notify_tx: &NotifyTx,
    narinfo_cache: &Arc<Mutex<NarinfoCache>>,
    config: &Config,
    reputation: &Arc<Mutex<ReputationTracker>>,
    conn_mgr: &Arc<Mutex<ConnectionManager>>,
    build_registry: &BuildRegistry,
    event_tx: &dashboard::EventBus,
    client: &reqwest::Client,
    nar_store: &Arc<Mutex<NarStore>>,
    local_peer_id: &str,
) -> anyhow::Result<()> {
    if config.dashboard_enabled {
        let state = dashboard::DashboardState {
            provider_cache: cache.clone(),
            reputation: reputation.clone(),
            conn_mgr: conn_mgr.clone(),
            build_registry: build_registry.clone(),
            transfer_registry: Arc::new(Mutex::new(HashMap::new())),
            event_history: Arc::new(Mutex::new(dashboard::EventHistory::default())),
            started: std::time::Instant::now(),
            peer_id: local_peer_id.to_string(),
            event_bus: event_tx.clone(),
            nar_store: nar_store.clone(),
            catalog: Arc::new(Mutex::new(HashMap::new())),
            cmd_tx: cmd_tx.clone(),
            seed_mutation_allowed: dashboard::seed_mutation_allowed_for_bind(
                &config.dashboard_bind,
            ),
        };
        let port = config.dashboard_port;
        let bind = config.dashboard_bind.clone();
        let bind_clone = bind.clone();
        tokio::spawn(async move {
            dashboard::serve(state, port, &bind_clone).await;
        });
        tracing::info!("Dashboard enabled on http://{}:{}", bind, port);
    }

    start_provider_health_monitor(
        cache.clone(),
        cmd_tx.clone(),
        config.clone(),
        event_tx.clone(),
        nar_store.clone(),
    );

    // Start Unix socket listener for relay connections
    let socket_path = config.socket_path.clone();
    start_socket_listener(
        &socket_path,
        cache,
        cmd_tx,
        query_tx,
        notify_tx,
        narinfo_cache,
        config,
        reputation,
        conn_mgr,
        build_registry,
        event_tx,
        client,
        nar_store,
    )
    .await?;

    Ok(())
}

fn start_provider_health_monitor(
    cache: ProviderCache,
    cmd_tx: UnboundedSender<SwarmCommand>,
    config: Config,
    event_tx: dashboard::EventBus,
    nar_store: Arc<Mutex<NarStore>>,
) {
    tokio::spawn(async move {
        let interval_secs = config.health_check_interval_secs.max(30);
        let mut tick = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

        loop {
            tick.tick().await;
            let hashes = nar_store.lock().unwrap().seeded_hashes();
            if hashes.is_empty() {
                continue;
            }

            for hash in &hashes {
                let _ = cmd_tx.send(SwarmCommand::GetProviders { hash: hash.clone() });
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

            let snapshot = cache.lock().await.clone();
            for hash in hashes {
                let provider_count = snapshot.get(&hash).map_or(0, Vec::len);
                let _ = event_tx.send(DashboardEvent::ProvidersFound {
                    nar_hash: hash.clone(),
                    provider_count,
                });

                if provider_count < config.min_providers {
                    tracing::warn!(
                        "Seeded nar {}.. has only {}/{} known providers; re-announcing local seed",
                        &hash[..16.min(hash.len())],
                        provider_count,
                        config.min_providers
                    );
                    let _ = cmd_tx.send(SwarmCommand::StartProviding { hash });
                }
            }
        }
    });
}

/// Start the Unix domain socket listener that accepts relay connections.
#[allow(clippy::too_many_arguments)]
async fn start_socket_listener(
    socket_path: &str,
    cache: &ProviderCache,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    query_tx: &UnboundedSender<String>,
    notify_tx: &NotifyTx,
    narinfo_cache: &Arc<Mutex<NarinfoCache>>,
    config: &Config,
    reputation: &Arc<Mutex<ReputationTracker>>,
    conn_mgr: &Arc<Mutex<ConnectionManager>>,
    build_registry: &BuildRegistry,
    event_tx: &dashboard::EventBus,
    client: &reqwest::Client,
    nar_store: &Arc<Mutex<NarStore>>,
) -> anyhow::Result<()> {
    // Remove stale socket file if present
    let _ = std::fs::remove_file(socket_path);

    let listener = tokio::net::UnixListener::bind(socket_path)
        .map_err(|e| anyhow::anyhow!("failed to bind socket {}: {}", socket_path, e))?;
    tracing::info!("Listening for relay connections on {}", socket_path);

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                tracing::info!("Relay connection accepted");
                let notify_rx = notify_tx.subscribe();
                let cache = cache.clone();
                let cmd_tx = cmd_tx.clone();
                let query_tx = query_tx.clone();
                let narinfo_cache = narinfo_cache.clone();
                let config = config.clone();
                let reputation = reputation.clone();
                let conn_mgr = conn_mgr.clone();
                let build_registry = build_registry.clone();
                let event_tx = event_tx.clone();
                let client = client.clone();
                let nar_store = nar_store.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_socket_connection(
                        stream,
                        notify_rx,
                        &cache,
                        &cmd_tx,
                        &query_tx,
                        &narinfo_cache,
                        &config,
                        &reputation,
                        &conn_mgr,
                        &build_registry,
                        &event_tx,
                        &client,
                        &nar_store,
                    )
                    .await
                    {
                        tracing::warn!("Socket connection error: {}", e);
                    }
                });
            },
            Err(e) => {
                tracing::warn!("Failed to accept socket connection: {}", e);
            },
        }
    }
}

/// Handle a single relay connection over a Unix socket.
#[allow(clippy::too_many_arguments)]
async fn handle_socket_connection(
    stream: tokio::net::UnixStream,
    notify_rx: NotifyRx,
    cache: &ProviderCache,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    query_tx: &UnboundedSender<String>,
    narinfo_cache: &Arc<Mutex<NarinfoCache>>,
    config: &Config,
    reputation: &Arc<Mutex<ReputationTracker>>,
    conn_mgr: &Arc<Mutex<ConnectionManager>>,
    build_registry: &BuildRegistry,
    event_tx: &dashboard::EventBus,
    client: &reqwest::Client,
    nar_store: &Arc<Mutex<NarStore>>,
) -> anyhow::Result<()> {
    use tokio::io::AsyncBufReadExt;

    let (socket_read, mut socket_write) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(socket_read);
    let mut notify_rx = notify_rx;

    // Read mode header: "mode: query" or "mode: substitute"
    let mut mode_line = String::new();
    let n = reader.read_line(&mut mode_line).await?;
    if n == 0 {
        return Err(anyhow::anyhow!("relay connection closed before sending mode header"));
    }
    let mode = mode_line.trim();

    tracing::info!("Relay mode: {}", mode);

    match mode {
        "mode: query" => {
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }

                let cmd = parse_command_line(line.trim())?;
                let mut reply = ReplyWriter::socket();

                match cmd {
                    DaemonCommand::Have(paths) => {
                        handle_have(
                            cache,
                            query_tx,
                            &mut reply,
                            &paths,
                            config,
                            &mut notify_rx,
                            narinfo_cache,
                            client,
                            Some(event_tx),
                        )
                        .await;
                    },
                    DaemonCommand::Info(paths) => {
                        handle_info(
                            config,
                            query_tx,
                            &mut notify_rx,
                            narinfo_cache,
                            &mut reply,
                            &paths,
                            client,
                            Some(event_tx),
                        )
                        .await;
                    },
                    DaemonCommand::Substitute { .. } => {},
                }

                if reply.has_data() {
                    reply.flush_socket(&mut socket_write).await?;
                }

                while notify_rx.try_recv().is_ok() {}
            }
            Ok(())
        },
        "mode: substitute" => {
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }

                let cmd = parse_command_line(line.trim())?;

                if let DaemonCommand::Substitute { path, dest } = cmd {
                    let mut reply = ReplyWriter::socket();

                    try_swarm_substitute(
                        config,
                        cache,
                        cmd_tx,
                        &mut reply,
                        &path,
                        &dest,
                        &mut notify_rx,
                        narinfo_cache,
                        reputation,
                        conn_mgr,
                        build_registry,
                        event_tx,
                        client,
                        nar_store,
                    )
                    .await;

                    if reply.has_data() {
                        reply.flush_socket(&mut socket_write).await?;
                    }
                }
            }
            Ok(())
        },
        other => Err(anyhow::anyhow!("unknown relay mode: {}", other)),
    }
}

#[allow(dead_code)]
async fn read_command_async() -> std::io::Result<DaemonCommand> {
    tokio::task::spawn_blocking(read_command).await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_have_command() {
        let cmd = parse_command_line("have /gnu/store/abc-foo /gnu/store/def-bar").unwrap();
        match cmd {
            DaemonCommand::Have(paths) => {
                assert_eq!(paths.len(), 2);
                assert_eq!(paths[0], "/gnu/store/abc-foo");
                assert_eq!(paths[1], "/gnu/store/def-bar");
            },
            _ => panic!("expected Have"),
        }
    }

    #[test]
    fn test_parse_substitute_command() {
        let cmd = parse_command_line("substitute /gnu/store/abc-foo /tmp/dest").unwrap();
        match cmd {
            DaemonCommand::Substitute { path, dest } => {
                assert_eq!(path, "/gnu/store/abc-foo");
                assert_eq!(dest, "/tmp/dest");
            },
            _ => panic!("expected Substitute"),
        }
    }

    #[test]
    fn test_parse_info_multiple_paths() {
        let cmd = parse_command_line("info /gnu/store/abc-foo /gnu/store/def-bar").unwrap();
        match cmd {
            DaemonCommand::Info(paths) => {
                assert_eq!(paths, vec!["/gnu/store/abc-foo", "/gnu/store/def-bar"]);
            },
            _ => panic!("expected Info"),
        }
    }

    #[test]
    fn test_fallback_plain_path() {
        let cmd = parse_command_line("/gnu/store/abc-foo").unwrap();
        match cmd {
            DaemonCommand::Have(paths) => {
                assert_eq!(paths.len(), 1);
                assert_eq!(paths[0], "/gnu/store/abc-foo");
            },
            _ => panic!("expected Have fallback"),
        }
    }

    #[test]
    fn test_format_trace_roundtrip() {
        let trace = format_trace_started("/a", "http://b", 100);
        assert!(trace.starts_with("@ download-started"));

        let trace = format_trace_succeeded("/a", "http://b", 100);
        assert!(trace.starts_with("@ download-succeeded"));
    }

    #[test]
    fn test_extract_hash_part() {
        assert_eq!(
            extract_hash_part("/gnu/store/abc123def456ghi789jkl012mno345pq-foo-1.0").unwrap(),
            "abc123def456ghi789jkl012mno345pq"
        );
    }

    #[test]
    fn test_extract_nar_hash_bytes_decodes_guix_nix_base32() {
        let bytes =
            extract_nar_hash_bytes("sha256:0qhasy0w9w9mfv0vacgzymxl4nww8cslyza5x2ci42v7i2b13lyl")
                .unwrap();
        assert_eq!(
            hex::encode(bytes),
            "d4d3119688670b1299e8457d4f35439c5b427bf5ff31b5c17635f1c481d70a62"
        );
    }

    #[test]
    fn test_extract_nar_hash_bytes_accepts_hex() {
        let hash = "d4d3119688670b1299e8457d4f35439c5b427bf5ff31b5c17635f1c481d70a62";
        let bytes = extract_nar_hash_bytes(&format!("sha256:{hash}")).unwrap();
        assert_eq!(hex::encode(bytes), hash);
    }

    #[test]
    fn test_guix_store_path_prefixes_narinfo_basenames() {
        assert_eq!(
            guix_store_path("x0qpkx4qcd7pzn121bg5plm67jf0icbz-gash-utils-0.2.0.tar.gz.drv"),
            "/gnu/store/x0qpkx4qcd7pzn121bg5plm67jf0icbz-gash-utils-0.2.0.tar.gz.drv"
        );
        assert_eq!(
            guix_store_path("/gnu/store/cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2"),
            "/gnu/store/cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2"
        );
        assert_eq!(guix_store_path(""), "");
    }

    #[test]
    fn test_extract_bad() {
        assert!(extract_hash_part("/bad/path").is_err());
    }

    #[tokio::test]
    async fn wait_for_providers_collects_matching_notifications() {
        let (notify_tx, mut notify_rx) = tokio::sync::broadcast::channel(8);
        let dht_key = "d4d3119688670b1299e8457d4f35439c5b427bf5ff31b5c17635f1c481d70a62";
        let stale = PeerId::random();
        let current = PeerId::random();
        let ignored = PeerId::random();

        notify_tx
            .send(SwarmNotification::ProvidersFound {
                hash: dht_key.to_string(),
                peers: vec![stale],
            })
            .unwrap();
        notify_tx
            .send(SwarmNotification::ProvidersFound {
                hash: "other".to_string(),
                peers: vec![ignored],
            })
            .unwrap();
        notify_tx
            .send(SwarmNotification::ProvidersFound {
                hash: dht_key.to_string(),
                peers: vec![stale, current],
            })
            .unwrap();

        let providers = wait_for_providers_for_duration(
            &mut notify_rx,
            dht_key,
            tokio::time::Duration::from_millis(50),
        )
        .await;

        assert_eq!(providers, vec![stale, current]);
    }

    #[test]
    fn select_provider_candidates_skips_banned_peers() {
        let good = PeerId::random();
        let banned = PeerId::random();
        let mut tracker = ReputationTracker::new(2);
        tracker.record_failure(banned);
        tracker.record_failure(banned);
        tracker.record_success(good, 1024);

        let reputation = Arc::new(Mutex::new(tracker));
        let conn_mgr = Arc::new(Mutex::new(ConnectionManager::new(Default::default())));

        let selected = select_provider_candidates(&[banned, good], &reputation, &conn_mgr, 8);

        assert_eq!(selected, vec![good]);
    }

    #[test]
    fn select_provider_candidates_respects_connection_backoff() {
        let stale = PeerId::random();
        let current = PeerId::random();
        let reputation = Arc::new(Mutex::new(ReputationTracker::new(5)));
        let conn_mgr =
            Arc::new(Mutex::new(ConnectionManager::new(crate::connection::ConnectionConfig {
                max_retries: 1,
                ..Default::default()
            })));

        conn_mgr.lock().unwrap().record_attempt(stale);

        let selected = select_provider_candidates(&[stale, current], &reputation, &conn_mgr, 8);

        assert_eq!(selected, vec![current]);
    }

    #[test]
    fn merge_provider_lists_deduplicates_in_order() {
        let first = PeerId::random();
        let second = PeerId::random();
        let third = PeerId::random();

        let merged = merge_provider_lists(&[first, second], &[second, third]);

        assert_eq!(merged, vec![first, second, third]);
    }
}
