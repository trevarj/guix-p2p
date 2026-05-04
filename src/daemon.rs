use std::{
    collections::HashMap,
    io::{self, BufRead},
    os::unix::io::RawFd,
    path::PathBuf,
};

use libp2p::PeerId;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    channel::{SwarmCommand, SwarmNotification},
    config::Config,
    dht::ProviderCache,
    narinfo::NarinfoCache,
    swarm::{
        block::BlockInfo,
        codec::{BlockData, BlockRequest, BlockResponse},
        downloader::{ActiveDownload, DownloadError},
    },
};

pub enum DaemonCommand {
    Have(Vec<String>),
    Info(String),
    Substitute { path: String, dest: String },
}

pub fn read_command() -> io::Result<DaemonCommand> {
    let mut line = String::new();
    let n = io::stdin().lock().read_line(&mut line)?;
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "stdin closed"));
    }
    parse_command_line(line.trim())
}

fn parse_command_line(line: &str) -> io::Result<DaemonCommand> {
    if let Some(rest) = line.strip_prefix("have ") {
        let paths = rest.split_whitespace().map(str::to_string).collect();
        Ok(DaemonCommand::Have(paths))
    } else if let Some(rest) = line.strip_prefix("info ") {
        Ok(DaemonCommand::Info(rest.to_string()))
    } else if let Some(rest) = line.strip_prefix("substitute ") {
        let mut parts = rest.splitn(2, ' ');
        let path = parts.next().unwrap_or("").to_string();
        let dest = parts.next().unwrap_or("").to_string();
        Ok(DaemonCommand::Substitute { path, dest })
    } else {
        Ok(DaemonCommand::Have(vec![line.to_string()]))
    }
}

pub struct ReplyWriter {
    fd: RawFd,
}

impl ReplyWriter {
    pub fn new() -> Self {
        ReplyWriter { fd: 4 }
    }

    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        let mut buf = line.as_bytes().to_vec();
        buf.push(b'\n');
        unsafe {
            let n = libc::write(self.fd, buf.as_ptr() as *const libc::c_void, buf.len());
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    pub fn write_end(&mut self) -> io::Result<()> {
        self.write_line("")
    }
}

pub fn extract_hash_part(store_path: &str) -> Result<String, String> {
    let prefix = "/gnu/store/";
    if !store_path.starts_with(prefix) {
        return Err(format!("not a store path: {}", store_path));
    }
    let rest = &store_path[prefix.len()..];
    let hash_end = rest.find('-').ok_or_else(|| format!("no name separator in: {}", store_path))?;
    Ok(rest[..hash_end].to_string())
}

/// Extract the raw SHA-256 bytes from a narinfo NarHash field.
fn extract_nar_hash_bytes(nar_hash: &str) -> [u8; 32] {
    let hex = nar_hash.strip_prefix("sha256:").unwrap_or(nar_hash);
    let bytes = hex::decode(hex).expect("valid hex hash in narinfo");
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes[..32]);
    arr
}

pub async fn run_query_mode(
    cache: &ProviderCache,
    query_tx: &UnboundedSender<String>,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    notify_rx: tokio::sync::mpsc::UnboundedReceiver<SwarmNotification>,
    narinfo_cache: &std::sync::Mutex<NarinfoCache>,
    config: &Config,
) -> anyhow::Result<()> {
    let _ = (cmd_tx, notify_rx);
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
                handle_have(cache, query_tx, &mut reply, &paths).await;
            },
            DaemonCommand::Info(path) => {
                handle_info(config, narinfo_cache, &mut reply, &path).await;
            },
            DaemonCommand::Substitute { .. } => {},
        }
    }

    Ok(())
}

async fn handle_have(
    cache: &ProviderCache,
    query_tx: &UnboundedSender<String>,
    reply: &mut ReplyWriter,
    paths: &[String],
) {
    for path in paths {
        let hash_part = match extract_hash_part(path) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("Skipping: {} ({})", path, e);
                continue;
            },
        };

        let _ = query_tx.send(hash_part.clone());
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        if crate::dht::has_providers(cache, &hash_part).await
            && let Err(e) = reply.write_line(path)
        {
            tracing::error!("have reply for {}: {}", path, e);
        }
    }

    if let Err(e) = reply.write_end() {
        tracing::error!("have end marker: {}", e);
    }
}

async fn handle_info(
    config: &Config,
    cache: &std::sync::Mutex<NarinfoCache>,
    reply: &mut ReplyWriter,
    path: &str,
) {
    let hash_part = match extract_hash_part(path) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("Cannot extract hash from {}: {}", path, e);
            let _ = reply.write_line(path);
            let _ = reply.write_end();
            return;
        },
    };

    match crate::http_client::fetch_narinfo(config, &hash_part, cache).await {
        Ok(info) => {
            let _ = reply.write_line(&info.store_path);
            let _ = reply.write_line(info.deriver.as_deref().unwrap_or(""));
            let _ = reply.write_line(&info.references.len().to_string());
            for r in &info.references {
                let _ = reply.write_line(r);
            }
            let download_size = info.urls.first().map(|u| u.file_size).unwrap_or(0);
            let _ = reply.write_line(&download_size.to_string());
            let _ = reply.write_line(&info.nar_size.to_string());
            let _ = reply.write_end();
        },
        Err(e) => {
            tracing::debug!("No narinfo for {}: {}", hash_part, e);
            let _ = reply.write_line(path);
            let _ = reply.write_end();
        },
    }
}

pub async fn run_substitute_mode(
    cache: &ProviderCache,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    mut notify_rx: tokio::sync::mpsc::UnboundedReceiver<SwarmNotification>,
    narinfo_cache: &std::sync::Mutex<NarinfoCache>,
    config: &Config,
    _query_tx: &UnboundedSender<String>,
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
            try_swarm_substitute(
                config,
                cache,
                cmd_tx,
                &mut reply,
                &path,
                &dest,
                &mut notify_rx,
                narinfo_cache,
            )
            .await;
        }
    }

    Ok(())
}

/// Full swarm download of a nar: narinfo → DHT providers → handshake → download → verify.
#[allow(clippy::too_many_arguments)]
async fn try_swarm_substitute(
    config: &Config,
    _cache: &ProviderCache,
    cmd_tx: &UnboundedSender<SwarmCommand>,
    reply: &mut ReplyWriter,
    path: &str,
    dest: &str,
    notify_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SwarmNotification>,
    narinfo_cache: &std::sync::Mutex<NarinfoCache>,
) {
    let hash_part = match extract_hash_part(path) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Cannot extract hash from {}: {}", path, e);
            let _ = reply.write_line(&format!("not-found {}", path));
            return;
        },
    };

    let narinfo = match crate::http_client::fetch_narinfo(config, &hash_part, narinfo_cache).await {
        Ok(info) => info,
        Err(e) => {
            tracing::warn!("Cannot fetch narinfo for {}: {}", hash_part, e);
            let _ = reply.write_line(&format!("not-found {}", path));
            return;
        },
    };

    let store_path = narinfo.store_path.clone();
    let nar_size = narinfo.nar_size;
    let nar_hash = narinfo.nar_hash.clone();
    let nar_hash_bytes = extract_nar_hash_bytes(&nar_hash);

    tracing::info!(
        hash = %nar_hash,
        size = nar_size,
        "Downloading nar via swarm"
    );

    // Step 1: request DHT providers
    let dht_key = hex::encode(nar_hash_bytes);
    let _ = cmd_tx.send(SwarmCommand::GetProviders { hash: dht_key.clone() });

    let providers = wait_for_providers(notify_rx, &dht_key, config).await;

    if providers.len() < config.min_providers {
        tracing::info!(
            hash = %nar_hash,
            provider_count = providers.len(),
            threshold = config.min_providers,
            "Not enough swarm providers; replying not-found"
        );
        let _ = reply.write_line(&format!("not-found {}", path));
        return;
    }

    tracing::info!("Found {} swarm providers for {}", providers.len(), nar_hash);

    // Step 2: handshake with each provider to discover block availability
    let handshakes = handshake_with_providers(cmd_tx, notify_rx, &providers, nar_hash_bytes).await;

    if handshakes.is_empty() {
        tracing::warn!("No successful handshakes; replying not-found");
        let _ = reply.write_line(&format!("not-found {}", path));
        return;
    }

    // Step 3: request blocks from peers
    let dest_path = PathBuf::from(dest);
    let mut download_block_info = BlockInfo::from_file_size(nar_size, config.block_size);

    // Use block hashes from first peer's handshake, or compute empty placeholder
    if let Some(first) = handshakes.first() {
        download_block_info.set_hashes(first.block_hashes.clone());
    }

    match download_blocks_from_peers(
        cmd_tx,
        notify_rx,
        &handshakes,
        nar_size,
        download_block_info,
        &nar_hash,
        config,
    )
    .await
    {
        Ok((nar_data, verified_hash)) => {
            // Write verified nar to dest
            if let Err(e) = tokio::fs::write(&dest_path, &nar_data).await {
                tracing::error!("Failed to write nar to {}: {}", dest_path.display(), e);
                let _ = reply.write_line(&format!("not-found {}", path));
                return;
            }

            let size = nar_data.len() as u64;
            println!(
                "{}",
                format_trace_succeeded(&store_path, &format!("p2p://{}", hash_part), size)
            );

            let _ = reply.write_line(&format!("success {} {}", verified_hash, size));
            tracing::info!("Swarm download succeeded for {}", store_path);
        },
        Err(e) => {
            tracing::error!("Swarm download failed for {}: {}", store_path, e);
            let _ = reply.write_line(&format!("not-found {}", path));
        },
    }
}

/// Wait for provider notifications for the given DHT key, with a timeout.
async fn wait_for_providers(
    notify_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SwarmNotification>,
    dht_key: &str,
    config: &Config,
) -> Vec<PeerId> {
    let deadline =
        tokio::time::Instant::now() + tokio::time::Duration::from_secs(config.request_timeout_secs);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            tracing::debug!("Provider lookup timed out for {}", dht_key);
            return Vec::new();
        }

        match tokio::time::timeout(tokio::time::Duration::from_secs(1), notify_rx.recv()).await {
            Ok(Some(SwarmNotification::ProvidersFound { hash, peers })) => {
                if hash == dht_key {
                    return peers;
                }
            },
            Ok(Some(_)) => {},
            Ok(None) => return Vec::new(),
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
    notify_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SwarmNotification>,
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
            Ok(Some(SwarmNotification::BlockResponse {
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
            Ok(None) => break,
            Err(_elapsed) => continue,
            _ => {},
        }
    }

    results
}

/// Orchestrate block downloads from peers.
async fn download_blocks_from_peers(
    cmd_tx: &UnboundedSender<SwarmCommand>,
    notify_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SwarmNotification>,
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
            let request = BlockRequest::GetBlocks { indices };
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
            Ok(Some(SwarmNotification::BlockResponse {
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
            Ok(None) => break,
            Err(_elapsed) => continue,
            _ => {},
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

#[allow(dead_code)]
pub fn format_trace_started(store_path: &str, url: &str, size: u64) -> String {
    format!("@ download-started {} {} {}", store_path, url, size)
}

#[allow(dead_code)]
pub fn format_trace_progress(store_path: &str, url: &str, total: u64, transferred: u64) -> String {
    format!("@ download-progress {} {} {} {}", store_path, url, total, transferred)
}

pub fn format_trace_succeeded(store_path: &str, url: &str, size: u64) -> String {
    format!("@ download-succeeded {} {} {}", store_path, url, size)
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
    fn test_extract_bad() {
        assert!(extract_hash_part("/bad/path").is_err());
    }
}
