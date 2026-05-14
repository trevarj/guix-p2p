use std::{
    collections::HashMap,
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
                // Always claim we have it; we can serve via HTTP fallback if
                // no P2P providers exist.
                tracing::info!("have: claiming {} (policy={:?})", path, config.substitute_policy);
                if let Err(e) = reply.write_line(path) {
                    tracing::error!("have reply for {}: {}", path, e);
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

        if let Some(tx) = event_tx
            && config.substitute_policy != SubstitutePolicy::P2pOnly
        {
            let p2p_available = crate::dht::has_providers(cache, &hash_part).await;
            let _ = tx.send(DashboardEvent::CatalogEntry {
                hash_part: hash_part.clone(),
                store_path: Some(path.clone()),
                nar_size: None,
                nar_hash: None,
                p2p_available,
            });
        }
    }

    if let Err(e) = reply.write_end() {
        tracing::error!("have end marker: {}", e);
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_info(
    config: &Config,
    query_tx: &UnboundedSender<String>,
    notify_rx: &mut NotifyRx,
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

        match crate::http_client::fetch_narinfo(config, &hash_part, cache, client).await {
            Ok(info) => {
                let nar_hash_bytes = match extract_nar_hash_bytes(&info.nar_hash) {
                    Some(bytes) => bytes,
                    None => {
                        tracing::warn!("info: skipping {} (invalid nar hash)", path);
                        continue;
                    },
                };
                let dht_key = hex::encode(nar_hash_bytes);
                if config.substitute_policy == SubstitutePolicy::P2pOnly {
                    let _ = query_tx.send(dht_key.clone());
                    let query_timeout =
                        tokio::time::Duration::from_secs(config.request_timeout_secs.min(5));
                    let providers =
                        wait_for_providers_for_duration(notify_rx, &dht_key, query_timeout).await;
                    if providers.len() < config.min_providers {
                        tracing::info!(
                            "info: skipping {} (p2p-only, only {}/{}) providers found",
                            path,
                            providers.len(),
                            config.min_providers
                        );
                        continue;
                    }
                }

                if let Some(tx) = event_tx {
                    let _ = tx.send(DashboardEvent::CatalogEntry {
                        hash_part: hash_part.clone(),
                        store_path: Some(info.store_path.clone()),
                        nar_size: Some(info.nar_size),
                        nar_hash: Some(dht_key),
                        p2p_available: config.substitute_policy == SubstitutePolicy::P2pOnly,
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
    _cache: &ProviderCache,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    reply: &mut ReplyWriter,
    path: &str,
    dest: &str,
    notify_rx: &mut NotifyRx,
    narinfo_cache: &Arc<Mutex<NarinfoCache>>,
    reputation: &Arc<Mutex<ReputationTracker>>,
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

    let narinfo =
        match crate::http_client::fetch_narinfo(config, &hash_part, narinfo_cache, client).await {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!("Cannot fetch narinfo for {}: {}", hash_part, e);
                let _ = reply.write_line("not-found");
                return;
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
                cmd_tx,
                notify_rx,
                &nar_hash_hex,
                &nar_hash_bytes,
                &store_path,
                nar_size,
                &hash_part,
                reputation,
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
                cmd_tx,
                notify_rx,
                &nar_hash_hex,
                &nar_hash_bytes,
                &store_path,
                nar_size,
                &hash_part,
                reputation,
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
                        cmd_tx,
                        notify_rx,
                        &nar_hash_hex,
                        &nar_hash_bytes,
                        &store_path,
                        nar_size,
                        &hash_part,
                        reputation,
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
    cmd_tx: &UnboundedSender<SwarmCommand>,
    notify_rx: &mut NotifyRx,
    nar_hash: &str,
    nar_hash_bytes: &[u8; 32],
    store_path: &str,
    nar_size: u64,
    _hash_part: &str,
    reputation: &Arc<Mutex<ReputationTracker>>,
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

    let providers = wait_for_providers(notify_rx, &dht_key, config).await;

    if providers.len() < config.min_providers {
        return Err(format!(
            "not enough P2P providers ({}/{})",
            providers.len(),
            config.min_providers
        ));
    }

    tracing::info!("Found {} P2P providers for {}", providers.len(), nar_hash);

    let mut providers = providers;
    reputation.lock().unwrap().sort_by_score(&mut providers);

    let handshakes = handshake_with_providers(cmd_tx, notify_rx, &providers, *nar_hash_bytes).await;

    if handshakes.is_empty() {
        return Err("no successful P2P handshakes".into());
    }

    let mut download_block_info = BlockInfo::from_file_size(nar_size, config.block_size);
    if let Some(first) = handshakes.first() {
        download_block_info.set_hashes(first.block_hashes.clone());
    }

    let download_start = std::time::Instant::now();

    match download_blocks_from_peers(
        cmd_tx,
        notify_rx,
        &handshakes,
        nar_size,
        download_block_info,
        nar_hash,
        config,
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
    notify_rx: &mut NotifyRx,
    dht_key: &str,
    config: &Config,
) -> Vec<PeerId> {
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

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            tracing::debug!("Provider lookup timed out for {}", dht_key);
            return Vec::new();
        }

        match tokio::time::timeout(tokio::time::Duration::from_secs(1), notify_rx.recv()).await {
            Ok(Ok(SwarmNotification::ProvidersFound { hash, peers })) => {
                if hash == dht_key {
                    return peers;
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

/// Handshake results: for each peer, which blocks they have.
struct PeerHandshake {
    peer: PeerId,
    #[allow(dead_code)]
    blocks_available: Vec<u32>,
    block_hashes: Vec<[u8; 32]>,
    #[allow(dead_code)]
    block_count: u32,
}

/// Send Handshake requests and collect replies.
async fn handshake_with_providers(
    cmd_tx: &UnboundedSender<SwarmCommand>,
    notify_rx: &mut NotifyRx,
    providers: &[PeerId],
    nar_hash_bytes: [u8; 32],
) -> Vec<PeerHandshake> {
    let max = providers.len().min(8);
    let mut results = Vec::new();
    let mut pending = HashMap::new();

    // Send handshakes
    for (i, peer) in providers.iter().take(max).enumerate() {
        let request = BlockRequest::Handshake { nar_hash: nar_hash_bytes.to_vec() };
        let _ = cmd_tx.send(SwarmCommand::SendBlockRequest { peer: *peer, request });

        // Use the index as a lightweight request-id for tracking
        pending.insert(i as u64, *peer);
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
                pending.retain(|_, p| *p != peer);
            },
            Ok(Ok(_)) => {},
            Ok(Err(_)) => break,
            Err(_elapsed) => continue,
        }
    }

    results
}

/// Orchestrate block downloads from peers.
async fn download_blocks_from_peers(
    cmd_tx: &UnboundedSender<SwarmCommand>,
    notify_rx: &mut NotifyRx,
    handshakes: &[PeerHandshake],
    nar_size: u64,
    block_info: BlockInfo,
    nar_hash: &str,
    config: &Config,
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
    let peer_count = handshakes.len();
    let blocks_per_peer = total.div_ceil(peer_count);

    for (i, hs) in handshakes.iter().enumerate() {
        let start = i * blocks_per_peer;
        let end = (start + blocks_per_peer).min(total);
        let indices: Vec<u32> = (start as u32..end as u32).collect();

        if !indices.is_empty() {
            let n = indices.len();
            let request = BlockRequest::GetBlocks { nar_hash: nar_hash_bytes(nar_hash), indices };
            tracing::debug!("Requested {} blocks from {}", n, hs.peer);
            let _ = cmd_tx.send(SwarmCommand::SendBlockRequest { peer: hs.peer, request });
        }
    }

    let overall_deadline =
        tokio::time::Instant::now() + tokio::time::Duration::from_secs(config.request_timeout_secs);
    let mut stall_deadline =
        tokio::time::Instant::now() + tokio::time::Duration::from_secs(config.stall_timeout_secs);
    let mut last_block_count = 0_usize;

    loop {
        let remaining = overall_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            tracing::warn!("Block download timed out");
            break;
        }

        if tokio::time::Instant::now() > stall_deadline {
            tracing::warn!(
                "Block download stalled (no new blocks for {}s)",
                config.stall_timeout_secs
            );
            break;
        }

        match tokio::time::timeout(tokio::time::Duration::from_secs(1), notify_rx.recv()).await {
            Ok(Ok(SwarmNotification::BlockResponse {
                peer,
                response: BlockResponse::Blocks { data },
            })) => {
                let blocks: Vec<(u32, Vec<u8>)> =
                    data.into_iter().map(|bd: BlockData| (bd.index, bd.data)).collect();
                download.record_blocks(peer, &blocks);

                let current_count: usize = download
                    .receivers
                    .values()
                    .map(|r| r.blocks.iter().filter(|b| b.is_some()).count())
                    .sum();
                if current_count > last_block_count {
                    last_block_count = current_count;
                    stall_deadline = tokio::time::Instant::now()
                        + tokio::time::Duration::from_secs(config.stall_timeout_secs);
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
                let _conn_mgr = conn_mgr.clone();
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
}
