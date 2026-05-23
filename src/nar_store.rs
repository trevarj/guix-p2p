use std::{
    collections::HashMap,
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

use anyhow::Context;
use sha2::{Digest, Sha256};
use tracing;

use crate::{
    nar_hash,
    narinfo::Narinfo,
    swarm::{
        block::{BlockInfo, compute_block_hashes},
        codec::{BlockData, BlockRequest, BlockResponse},
    },
};

/// Manages locally cached nar data for serving blocks to peers.
///
/// Nars are stored on disk at `<cache_dir>/nar/<sha256hex>.nar`.
/// On startup, the store scans this directory and indexes all files by their
/// filename (which must be the hex-encoded sha256 of the nar content).
/// After a successful swarm download, the nar is saved here so it can be
/// re-seeded to other peers.
pub struct NarStore {
    cache_dir: PathBuf,
    block_size: usize,
    /// In-memory index: nar_hash_hex -> local nar data and optional store path metadata.
    index: HashMap<String, NarEntry>,
}

struct NarEntry {
    path: PathBuf,
    nar_size: u64,
    block_info: BlockInfo,
    store_path: Option<String>,
    source: SeedSource,
}

/// Why a NAR is available for serving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedSource {
    /// Seeded explicitly from a store path through config, CLI, or dashboard.
    Manual,
    /// Cached after a successful substitute download.
    Downloaded,
    /// Found in the cache at startup without persisted provenance.
    Cache,
}

impl SeedSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SeedSource::Manual => "manual",
            SeedSource::Downloaded => "downloaded",
            SeedSource::Cache => "cache",
        }
    }
}

/// Summary info for a seeded nar, used by the dashboard API.
#[derive(Debug, Clone)]
pub struct SeededNarInfo {
    pub nar_size: u64,
    pub block_count: u32,
    pub block_size: u32,
    pub store_path: Option<String>,
    pub source: SeedSource,
}

impl NarStore {
    pub fn new(cache_dir: &Path, block_size: usize) -> Self {
        let nar_dir = cache_dir.join("nar");
        let _ = std::fs::create_dir_all(&nar_dir);

        let mut store = NarStore { cache_dir: nar_dir, block_size, index: HashMap::new() };
        store.scan();
        store
    }

    /// Scan the nar directory and index all stored nars.
    fn scan(&mut self) {
        let entries = match std::fs::read_dir(&self.cache_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "nar") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };

            let data = match std::fs::read(&path) {
                Ok(data) => data,
                Err(e) => {
                    tracing::warn!("failed to read cached nar {}: {}", path.display(), e);
                    continue;
                },
            };
            let actual_hash = hex::encode(Sha256::digest(&data));
            if actual_hash != stem {
                tracing::warn!(
                    "skipping cached nar with hash mismatch: file={} expected={} actual={}",
                    path.display(),
                    stem,
                    actual_hash,
                );
                continue;
            }
            let nar_size = data.len() as u64;
            let block_info = BlockInfo::from_file_size(nar_size, self.block_size);

            tracing::info!(
                "indexed local nar: hash={}.. size={} blocks={}",
                &stem[..16.min(stem.len())],
                nar_size,
                block_info.block_count,
            );
            self.index.insert(
                stem,
                NarEntry {
                    path,
                    nar_size,
                    block_info,
                    store_path: None,
                    source: SeedSource::Cache,
                },
            );
        }

        tracing::info!("nar store: {} nars indexed", self.index.len());
    }

    /// Save a nar to the store after a successful download.
    pub fn save(&mut self, nar_hash_hex: &str, nar_data: &[u8]) -> anyhow::Result<()> {
        self.save_with_source(nar_hash_hex, nar_data, None, SeedSource::Downloaded)
    }

    /// Save a nar and retain the originating store path when it is known.
    pub fn save_with_store_path(
        &mut self,
        nar_hash_hex: &str,
        nar_data: &[u8],
        store_path: Option<String>,
    ) -> anyhow::Result<()> {
        self.save_with_source(nar_hash_hex, nar_data, store_path, SeedSource::Downloaded)
    }

    fn save_with_source(
        &mut self,
        nar_hash_hex: &str,
        nar_data: &[u8],
        store_path: Option<String>,
        source: SeedSource,
    ) -> anyhow::Result<()> {
        let path = self.cache_dir.join(format!("{}.nar", nar_hash_hex));
        let mut f = std::fs::File::create(&path).context("failed to create nar file")?;
        f.write_all(nar_data).context("failed to write nar data")?;

        let nar_size = nar_data.len() as u64;
        let block_info = BlockInfo::from_file_size(nar_size, self.block_size);

        tracing::info!(
            "saved nar: hash={}.. size={} blocks={}",
            &nar_hash_hex[..16.min(nar_hash_hex.len())],
            nar_size,
            block_info.block_count,
        );

        self.index.insert(
            nar_hash_hex.to_string(),
            NarEntry { path, nar_size, block_info, store_path, source },
        );
        Ok(())
    }

    /// Seed a single-item nar from a store path using Guix's nar serializer.
    /// The nar hash is computed via `guix hash -S nar -f hex`.
    pub fn seed_store_path(&mut self, store_path: &str) -> anyhow::Result<String> {
        let nar_hash_hex = compute_nar_hash(store_path).context("failed to compute nar hash")?;

        if self.index.contains_key(&nar_hash_hex) {
            if let Some(entry) = self.index.get_mut(&nar_hash_hex) {
                entry.store_path.get_or_insert_with(|| store_path.to_string());
                entry.source = SeedSource::Manual;
            }
            tracing::info!("nar already seeded: {}..", &nar_hash_hex[..16]);
            return Ok(nar_hash_hex);
        }

        let nar_data = export_nar(store_path).context("failed to export nar")?;
        self.save_with_source(
            &nar_hash_hex,
            &nar_data,
            Some(store_path.to_string()),
            SeedSource::Manual,
        )?;
        Ok(nar_hash_hex)
    }

    /// Return the list of nar hashes currently stored.
    pub fn seeded_hashes(&self) -> Vec<String> {
        self.index.keys().cloned().collect()
    }

    /// Attach store-path metadata from trusted narinfos to cached nars.
    pub fn annotate_from_narinfos(&mut self, narinfos: &[Narinfo]) -> usize {
        let mut updated = 0;
        for narinfo in narinfos {
            let Some(hash) = nar_hash::sha256_bytes(&narinfo.nar_hash).map(hex::encode) else {
                continue;
            };
            let Some(entry) = self.index.get_mut(&hash) else {
                continue;
            };
            if entry.store_path.is_none() && !narinfo.store_path.is_empty() {
                entry.store_path = Some(narinfo.store_path.clone());
                updated += 1;
            }
        }
        updated
    }

    /// Number of seeded nars.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Check if the store has no seeded nars.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Check if we have a nar for the given hash.
    pub fn has_nar(&self, nar_hash_hex: &str) -> bool {
        self.index.contains_key(nar_hash_hex)
    }

    /// Return summary info for a seeded nar (for dashboard).
    pub fn seed_info(&self, nar_hash_hex: &str) -> Option<SeededNarInfo> {
        let entry = self.index.get(nar_hash_hex)?;
        Some(SeededNarInfo {
            nar_size: entry.nar_size,
            block_count: entry.block_info.block_count,
            block_size: self.block_size as u32,
            store_path: entry.store_path.clone(),
            source: entry.source,
        })
    }

    /// Stop serving a nar and remove the cached nar file from disk.
    pub fn remove_seed(&mut self, nar_hash_hex: &str) -> Option<SeededNarInfo> {
        let entry = self.index.remove(nar_hash_hex)?;
        if let Err(e) = std::fs::remove_file(&entry.path) {
            tracing::warn!("failed to remove cached nar {}: {}", entry.path.display(), e);
        }
        Some(SeededNarInfo {
            nar_size: entry.nar_size,
            block_count: entry.block_info.block_count,
            block_size: self.block_size as u32,
            store_path: entry.store_path,
            source: entry.source,
        })
    }

    /// Handle an incoming block request. Returns None if we don't have this nar.
    pub fn handle_request(&self, request: &BlockRequest) -> Option<BlockResponse> {
        match request {
            BlockRequest::Handshake { nar_hash } => {
                let key = hex::encode(nar_hash);
                let entry = self.index.get(&key)?;

                // Compute block hashes from the file on disk
                let hashes = match self.read_block_hashes(&key) {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::warn!("failed to read block hashes for {}: {}", key, e);
                        return Some(BlockResponse::HandshakeReply {
                            blocks_available: vec![],
                            block_count: 0,
                            block_size: self.block_size as u32,
                            block_hashes: vec![],
                        });
                    },
                };

                let n = entry.block_info.block_count;
                let available: Vec<u32> = (0..n).collect();

                tracing::info!(
                    "handshake reply: hash={}.. blocks={}",
                    &key[..16.min(key.len())],
                    n,
                );

                Some(BlockResponse::HandshakeReply {
                    blocks_available: available,
                    block_count: n,
                    block_size: self.block_size as u32,
                    block_hashes: hashes.iter().map(|h| h.to_vec()).collect(),
                })
            },
            BlockRequest::GetBlocks { nar_hash, indices } => {
                let key = hex::encode(nar_hash);
                self.serve_blocks(&key, indices)
            },
        }
    }

    /// Handle a block request for a specific nar hash.
    pub fn handle_request_for_hash(
        &self,
        nar_hash_hex: &str,
        request: &BlockRequest,
    ) -> Option<BlockResponse> {
        match request {
            BlockRequest::Handshake { .. } => {
                let entry = self.index.get(nar_hash_hex)?;
                let hashes = match self.read_block_hashes(nar_hash_hex) {
                    Ok(h) => h,
                    Err(_) => return None,
                };
                let n = entry.block_info.block_count;
                Some(BlockResponse::HandshakeReply {
                    blocks_available: (0..n).collect(),
                    block_count: n,
                    block_size: self.block_size as u32,
                    block_hashes: hashes.iter().map(|h| h.to_vec()).collect(),
                })
            },
            BlockRequest::GetBlocks { indices, .. } => self.serve_blocks(nar_hash_hex, indices),
        }
    }

    fn serve_blocks(&self, nar_hash_hex: &str, indices: &[u32]) -> Option<BlockResponse> {
        let entry = self.index.get(nar_hash_hex)?;
        let data = std::fs::read(&entry.path).ok()?;

        let blks: Vec<BlockData> = indices
            .iter()
            .filter_map(|&i| {
                let off = i as usize * self.block_size;
                if off >= data.len() {
                    return None;
                }
                let end = (off + self.block_size).min(data.len());
                Some(BlockData { index: i, data: data[off..end].to_vec() })
            })
            .collect();

        if blks.is_empty() {
            Some(BlockResponse::Error { message: "no blocks available for this nar".into() })
        } else {
            tracing::info!(
                "serving {} block(s): hash={}..",
                blks.len(),
                &nar_hash_hex[..16.min(nar_hash_hex.len())],
            );
            Some(BlockResponse::Blocks { data: blks })
        }
    }

    fn read_block_hashes(&self, nar_hash_hex: &str) -> anyhow::Result<Vec<[u8; 32]>> {
        let entry = self
            .index
            .get(nar_hash_hex)
            .ok_or_else(|| anyhow::anyhow!("nar not found in index"))?;
        let data =
            std::fs::read(&entry.path).context("failed to read nar file for hash computation")?;
        Ok(compute_block_hashes(&data, self.block_size))
    }
}

/// Thread-safe wrapper for NarStore.
pub type SharedNarStore = Mutex<NarStore>;

/// Compute the nar hash of a store path using `guix hash -S nar -f hex`.
fn compute_nar_hash(store_path: &str) -> anyhow::Result<String> {
    let output = Command::new(system_profile_command("guix"))
        .args(["hash", "-S", "nar", "-f", "hex", store_path])
        .output()
        .context("failed to run `guix hash`")?;

    if !output.status.success() {
        anyhow::bail!(
            "guix hash failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Export raw single-item nar data from a store path.
///
/// `guix archive --export` intentionally writes a nar bundle with store-item
/// metadata and a signature. Substitute servers serve the raw single-item nar,
/// so use Guix's serializer directly.
fn export_nar(store_path: &str) -> anyhow::Result<Vec<u8>> {
    let mut command = Command::new(system_profile_command("guile"));
    add_system_profile_guile_env(&mut command);
    let output = command
        .args([
            "-c",
            r#"
(use-modules (guix serialization)
             (ice-9 match))
(match (command-line)
  ((_ store-path)
   (write-file store-path (current-output-port)))
  (_
   (format (current-error-port) "usage: guile -c SCRIPT STORE-PATH~%")
   (exit 2)))
"#,
            store_path,
        ])
        .output()
        .context("failed to run Guix nar serializer via `guile`")?;

    if !output.status.success() {
        anyhow::bail!(
            "Guix nar serializer failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(output.stdout)
}

fn system_profile_command(name: &str) -> OsString {
    system_profile_command_from(std::path::Path::new("/run/current-system/profile/bin"), name)
}

fn system_profile_command_from(bin_dir: &std::path::Path, name: &str) -> OsString {
    let command = bin_dir.join(name);
    if command.exists() { command.into_os_string() } else { OsString::from(name) }
}

fn add_system_profile_guile_env(command: &mut Command) {
    for (key, value) in
        system_profile_guile_env(std::path::Path::new("/run/current-system/profile"))
    {
        command.env(key, value);
    }
}

fn system_profile_guile_env(profile: &Path) -> Vec<(&'static str, OsString)> {
    vec![
        ("GUILE_LOAD_PATH", profile.join("share/guile/site/3.0").into_os_string()),
        (
            "GUILE_LOAD_COMPILED_PATH",
            join_env_paths([
                profile.join("lib/guile/3.0/site-ccache"),
                profile.join("share/guile/site/3.0"),
            ]),
        ),
        ("GUILE_EXTENSIONS_PATH", profile.join("lib/guile/3.0/extensions").into_os_string()),
    ]
}

fn join_env_paths(paths: impl IntoIterator<Item = PathBuf>) -> OsString {
    std::env::join_paths(paths).unwrap_or_else(|_| OsString::new())
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn nar_store_resolves_system_profile_commands() {
        let command = system_profile_command("guix");

        assert!(!command.is_empty());
    }

    #[test]
    fn nar_store_prefers_system_profile_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let command_path = tmp.path().join("guile");
        std::fs::write(&command_path, b"").unwrap();

        let command = system_profile_command_from(tmp.path(), "guile");

        assert_eq!(command, command_path.into_os_string());
    }

    #[test]
    fn nar_store_falls_back_to_path_command_names() {
        let tmp = tempfile::tempdir().unwrap();
        let command = system_profile_command_from(tmp.path(), "guix");

        assert_eq!(command, std::ffi::OsString::from("guix"));
    }

    #[test]
    fn nar_store_sets_system_profile_guile_module_paths() {
        let env = system_profile_guile_env(Path::new("/system/profile"));

        assert!(env.iter().any(|(key, value)| {
            *key == "GUILE_LOAD_PATH"
                && value == &std::ffi::OsString::from("/system/profile/share/guile/site/3.0")
        }));
        assert!(env.iter().any(|(key, value)| {
            *key == "GUILE_LOAD_COMPILED_PATH"
                && value.to_string_lossy().contains("/system/profile/lib/guile/3.0/site-ccache")
        }));
        assert!(env.iter().any(|(key, value)| {
            *key == "GUILE_EXTENSIONS_PATH"
                && value == &std::ffi::OsString::from("/system/profile/lib/guile/3.0/extensions")
        }));
    }

    #[test]
    fn test_nar_store_save_and_query() {
        let tmp = tempfile::tempdir().unwrap();
        let data = vec![42u8; 1024];
        let hash = hex::encode(Sha256::digest(&data));

        let mut store = NarStore::new(tmp.path(), 512);
        assert!(!store.has_nar(&hash));

        store.save(&hash, &data).unwrap();
        assert!(store.has_nar(&hash));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_nar_store_persist_across_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let data = vec![0xABu8; 2048];
        let hash = hex::encode(Sha256::digest(&data));

        {
            let mut store = NarStore::new(tmp.path(), 512);
            store.save(&hash, &data).unwrap();
            assert_eq!(store.len(), 1);
        }

        // Reopen — should find the saved nar on disk
        let store = NarStore::new(tmp.path(), 512);
        assert!(store.has_nar(&hash));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_nar_store_skips_cached_hash_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let nar_dir = tmp.path().join("nar");
        std::fs::create_dir_all(&nar_dir).unwrap();

        let good_data = b"good nar bytes";
        let good_hash = hex::encode(Sha256::digest(good_data));
        std::fs::write(nar_dir.join(format!("{good_hash}.nar")), b"different bytes").unwrap();

        let store = NarStore::new(tmp.path(), 512);

        assert!(!store.has_nar(&good_hash));
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_nar_store_handshake_reply() {
        let tmp = tempfile::tempdir().unwrap();
        let data = vec![0xCCu8; 1500];
        let hash = hex::encode(Sha256::digest(&data));

        let mut store = NarStore::new(tmp.path(), 512);
        store.save(&hash, &data).unwrap();

        let request = BlockRequest::Handshake { nar_hash: hex::decode(&hash).unwrap() };
        let resp = store.handle_request(&request).unwrap();

        match resp {
            BlockResponse::HandshakeReply { blocks_available, block_count, block_size, .. } => {
                assert_eq!(block_count, 3); // ceil(1500/512)
                assert_eq!(blocks_available.len(), 3);
                assert_eq!(block_size, 512);
            },
            other => panic!("expected HandshakeReply, got {:?}", other),
        }
    }

    #[test]
    fn test_nar_store_get_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        let data = vec![0xDDu8; 1500];
        let hash = hex::encode(Sha256::digest(&data));

        let mut store = NarStore::new(tmp.path(), 512);
        store.save(&hash, &data).unwrap();

        let resp = store
            .handle_request_for_hash(
                &hash,
                &BlockRequest::GetBlocks {
                    nar_hash: hex::decode(&hash).unwrap(),
                    indices: vec![0, 2],
                },
            )
            .unwrap();

        match resp {
            BlockResponse::Blocks { data: blocks } => {
                assert_eq!(blocks.len(), 2);
                assert_eq!(blocks[0].index, 0);
                assert_eq!(blocks[0].data.len(), 512);
                assert_eq!(blocks[1].index, 2);
                assert_eq!(blocks[1].data.len(), 1500 - 1024); // last block
            },
            other => panic!("expected Blocks, got {:?}", other),
        }
    }

    #[test]
    fn test_nar_store_missing_nar_handshake() {
        let tmp = tempfile::tempdir().unwrap();
        let store = NarStore::new(tmp.path(), 512);

        let request = BlockRequest::Handshake { nar_hash: vec![0u8; 32] };
        assert!(store.handle_request(&request).is_none());
    }

    #[test]
    fn test_seeded_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let data = vec![1u8; 100];
        let hash = hex::encode(Sha256::digest(&data));

        let mut store = NarStore::new(tmp.path(), 512);
        store.save(&hash, &data).unwrap();

        let hashes = store.seeded_hashes();
        assert_eq!(hashes.len(), 1);
        assert_eq!(hashes[0], hash);
        assert_eq!(store.seed_info(&hash).unwrap().source, SeedSource::Downloaded);
    }

    #[test]
    fn scanned_nars_are_marked_as_cache_source() {
        let tmp = tempfile::tempdir().unwrap();
        let data = b"cached nar bytes";
        let hash = hex::encode(Sha256::digest(data));
        let nar_dir = tmp.path().join("nar");
        std::fs::create_dir_all(&nar_dir).unwrap();
        std::fs::write(nar_dir.join(format!("{hash}.nar")), data).unwrap();

        let store = NarStore::new(tmp.path(), 512);

        assert_eq!(store.seed_info(&hash).unwrap().source, SeedSource::Cache);
    }

    #[test]
    fn annotate_from_narinfos_attaches_store_path_to_cached_nar() {
        let tmp = tempfile::tempdir().unwrap();
        let data = vec![1u8; 100];
        let hash = hex::encode(Sha256::digest(&data));
        let store_path = "/gnu/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-foo".to_string();

        let mut store = NarStore::new(tmp.path(), 512);
        store.save(&hash, &data).unwrap();

        let updated = store.annotate_from_narinfos(&[Narinfo {
            store_path: store_path.clone(),
            nar_hash: format!("sha256:{hash}"),
            nar_size: data.len() as u64,
            references: vec![],
            deriver: None,
            urls: vec![],
            signed_portion: String::new(),
            signature: None,
        }]);

        assert_eq!(updated, 1);
        assert_eq!(store.seed_info(&hash).unwrap().store_path, Some(store_path));
    }
}
