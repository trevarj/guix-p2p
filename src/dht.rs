use std::{collections::HashMap, sync::Arc};

use libp2p::kad::{Event as KadEvent, GetProvidersOk, QueryResult};

use crate::{
    behaviour::GuixP2PBehaviour,
    channel::{NotifyTx, SwarmNotification},
};

pub type ProviderCache = Arc<tokio::sync::Mutex<HashMap<String, Vec<libp2p::PeerId>>>>;

pub fn create_provider_cache() -> ProviderCache {
    Arc::new(tokio::sync::Mutex::new(HashMap::new()))
}

pub async fn has_providers(cache: &ProviderCache, hash_part: &str) -> bool {
    let guard = cache.lock().await;
    guard.get(hash_part).map(|v| !v.is_empty()).unwrap_or(false)
}

#[allow(dead_code)]
pub async fn get_providers(cache: &ProviderCache, hash_part: &str) -> Vec<libp2p::PeerId> {
    let guard = cache.lock().await;
    guard.get(hash_part).cloned().unwrap_or_default()
}

pub fn extract_hash_bytes(hex_hash: &str) -> [u8; 32] {
    let hex = hex_hash.strip_prefix("sha256:").unwrap_or(hex_hash);
    match hex::decode(hex) {
        Ok(bytes) if bytes.len() >= 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes[..32]);
            arr
        },
        _ => {
            // The hash might not be a valid hex SHA-256 (e.g., a nix-base32 hash
            // from a store path). Use a best-effort approach: zero-pad or hash it.
            tracing::warn!("Invalid hex hash for DHT lookup: {}..", &hex_hash[..hex.len().min(16)]);
            let mut arr = [0u8; 32];
            let bytes = hex.as_bytes();
            let len = bytes.len().min(32);
            arr[..len].copy_from_slice(&bytes[..len]);
            arr
        },
    }
}

pub fn handle_kad_event(cache: &ProviderCache, notify_tx: &NotifyTx, event: &KadEvent) {
    let (key_hex, _key_bytes, providers) = match event {
        KadEvent::OutboundQueryProgressed {
            result:
                QueryResult::GetProviders(Ok(GetProvidersOk::FoundProviders { key, providers, .. })),
            ..
        } => {
            let raw = key.as_ref().to_vec();
            let k = hex::encode(&raw);
            let peers: Vec<libp2p::PeerId> = providers.iter().copied().collect();
            (k, raw, peers)
        },
        _ => return,
    };

    tracing::debug!("DHT found {} providers for {}", providers.len(), key_hex);

    // Insert synchronously — cache is an async mutex so we need to spawn,
    // but we also send the notification before the spawn completes. This is
    // fine because the notification is what drives the daemon logic; the
    // cache is secondary.
    let cache_clone = cache.clone();
    let hex_clone = key_hex.clone();
    let peers_clone = providers.clone();
    tokio::spawn(async move {
        let mut guard = cache_clone.lock().await;
        guard.insert(hex_clone, peers_clone);
    });

    let _ = notify_tx.send(SwarmNotification::ProvidersFound { hash: key_hex, peers: providers });
}

pub fn bootstrap(
    swarm: &mut libp2p::Swarm<GuixP2PBehaviour>,
    peers: &[String],
) -> anyhow::Result<()> {
    let mut connected = 0;
    let mut failed = 0;

    for addr_str in peers {
        match addr_str.parse::<libp2p::Multiaddr>() {
            Ok(addr) => {
                tracing::info!("Bootstrapping from {}", addr);
                match swarm.dial(addr) {
                    Ok(_) => connected += 1,
                    Err(e) => {
                        tracing::warn!("Failed to dial bootstrap peer {}: {}", addr_str, e);
                        failed += 1;
                    },
                }
            },
            Err(e) => {
                tracing::warn!("Invalid bootstrap address {}: {}", addr_str, e);
                failed += 1;
            },
        }
    }

    if connected == 0 && failed > 0 {
        tracing::warn!(
            "Failed to connect to any bootstrap peers ({} attempted). Node will start but may be \
             isolated.",
            failed
        );
    } else {
        tracing::info!("Bootstrap complete: {} connected, {} failed", connected, failed);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_provider_cache() {
        let cache = create_provider_cache();
        // Verify it's created (no panic)
        let _ = cache;
    }

    #[tokio::test]
    async fn test_has_providers_empty_cache() {
        let cache = create_provider_cache();
        assert!(!has_providers(&cache, "abcdef").await);
    }

    #[tokio::test]
    async fn test_has_providers_with_data() {
        let cache = create_provider_cache();
        let peer_id = libp2p::PeerId::random();
        {
            let mut guard = cache.lock().await;
            guard.insert("abcdef".to_string(), vec![peer_id]);
        }
        assert!(has_providers(&cache, "abcdef").await);
    }

    #[tokio::test]
    async fn test_has_providers_empty_vec() {
        let cache = create_provider_cache();
        {
            let mut guard = cache.lock().await;
            guard.insert("abcdef".to_string(), vec![]);
        }
        // Empty vec means no providers
        assert!(!has_providers(&cache, "abcdef").await);
    }

    #[tokio::test]
    async fn test_has_providers_different_key() {
        let cache = create_provider_cache();
        let peer_id = libp2p::PeerId::random();
        {
            let mut guard = cache.lock().await;
            guard.insert("abcdef".to_string(), vec![peer_id]);
        }
        assert!(!has_providers(&cache, "different").await);
    }
}
