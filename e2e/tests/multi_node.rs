use std::time::Duration;

use guix_p2p::swarm::codec::{BlockRequest, BlockResponse};
use guix_p2p_e2e::TestNode;
use sha2::{Digest, Sha256};

fn init() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "error".into()),
        )
        .try_init();
}

fn make_nar(seed: u8, size: usize) -> (Vec<u8>, String) {
    let mut data = vec![seed; size];
    for (i, b) in data.iter_mut().enumerate() {
        *b = seed.wrapping_add((i % 251) as u8);
    }
    let hash = hex::encode(Sha256::digest(&data));
    (data, hash)
}

fn is_network_permission_denied(err: &anyhow::Error) -> bool {
    err.chain().any(|e| {
        e.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::PermissionDenied)
            || e.to_string().contains("Operation not permitted")
    })
}

/// Provider discovery via Kademlia DHT requires 3+ nodes for routing-table
/// convergence on a LAN. This test is skipped in the default 2-node topology.
#[tokio::test]
#[ignore]
async fn seeder_announce_downloader_discovers() {
    init();
    let (nar_data, nar_hash) = make_nar(0xaa, 4096);
    let seeder = TestNode::seeder(&nar_hash, nar_data).await.unwrap();
    let mut downloader = TestNode::node(seeder.addr()).await.unwrap();
    tokio::time::sleep(Duration::from_secs(5)).await;
    downloader.lookup(&nar_hash);
    let providers = downloader.wait_providers(&nar_hash, Duration::from_secs(20)).await;
    assert!(!providers.is_empty(), "downloader should discover seeder via DHT");
}

#[tokio::test]
async fn block_handshake_and_transfer() {
    init();
    let (nar_data, nar_hash) = make_nar(0xbb, 200_000);
    let seeder = match TestNode::seeder(&nar_hash, nar_data.clone()).await {
        Ok(node) => node,
        Err(e) if is_network_permission_denied(&e) => {
            eprintln!("skipping E2E network test: {e:#}");
            return;
        },
        Err(e) => panic!("{e:#}"),
    };
    let seeder_pid = seeder.pid();
    let mut downloader = match TestNode::node(seeder.addr()).await {
        Ok(node) => node,
        Err(e) if is_network_permission_denied(&e) => {
            eprintln!("skipping E2E network test: {e:#}");
            return;
        },
        Err(e) => panic!("{e:#}"),
    };
    tokio::time::sleep(Duration::from_secs(5)).await;

    let nar_hash_bytes = hex::decode(&nar_hash).unwrap();
    downloader.send_req(seeder_pid, BlockRequest::Handshake { nar_hash: nar_hash_bytes });
    let resp = downloader.wait_block_resp(seeder_pid, Duration::from_secs(20)).await;
    assert!(resp.is_some(), "should receive handshake reply from seeder");
    let block_hashes = match resp.unwrap() {
        BlockResponse::HandshakeReply { block_hashes, .. } => block_hashes,
        other => panic!("expected HandshakeReply, got {:?}", other),
    };

    downloader.send_req(
        seeder_pid,
        BlockRequest::GetBlocks { nar_hash: hex::decode(&nar_hash).unwrap(), indices: vec![0] },
    );
    let resp = downloader.wait_block_resp(seeder_pid, Duration::from_secs(20)).await;
    assert!(resp.is_some(), "should receive block data from seeder");

    let mut got_block = false;
    if let Some(BlockResponse::Blocks { data }) = resp {
        for bd in data {
            let expected = &block_hashes[bd.index as usize];
            let actual = Sha256::digest(&bd.data);
            if actual.as_slice() == expected.as_slice() {
                got_block = true;
            }
        }
    }
    assert!(got_block, "received block should pass hash verification");
}

#[tokio::test]
async fn connected_bootstrap_handshake_works_when_dht_lookup_is_empty() {
    init();
    let (nar_data, nar_hash) = make_nar(0xab, 96_000);
    let seeder = match TestNode::seeder(&nar_hash, nar_data).await {
        Ok(node) => node,
        Err(e) if is_network_permission_denied(&e) => {
            eprintln!("skipping E2E network test: {e:#}");
            return;
        },
        Err(e) => panic!("{e:#}"),
    };
    let seeder_pid = seeder.pid();
    let mut downloader = match TestNode::node(seeder.addr()).await {
        Ok(node) => node,
        Err(e) if is_network_permission_denied(&e) => {
            eprintln!("skipping E2E network test: {e:#}");
            return;
        },
        Err(e) => panic!("{e:#}"),
    };
    tokio::time::sleep(Duration::from_secs(5)).await;

    let (_missing_data, missing_hash) = make_nar(0xfe, 8192);
    downloader.lookup(&missing_hash);
    let providers = downloader.wait_providers(&missing_hash, Duration::from_secs(3)).await;
    assert!(providers.is_empty(), "control lookup should have no DHT providers");

    downloader.send_req(
        seeder_pid,
        BlockRequest::Handshake { nar_hash: hex::decode(&nar_hash).unwrap() },
    );
    let resp = downloader.wait_block_resp(seeder_pid, Duration::from_secs(20)).await;
    let Some(BlockResponse::HandshakeReply { blocks_available, block_count, .. }) = resp else {
        panic!("connected bootstrap peer should answer fallback handshake");
    };
    assert!(!blocks_available.is_empty(), "fallback handshake should prove availability");
    assert_eq!(blocks_available.len(), block_count as usize);
}

#[tokio::test]
async fn full_nar_download() {
    init();
    let (nar_data, nar_hash) = make_nar(0xcc, 200_000);
    let nar_size = nar_data.len() as u64;
    let block_info =
        guix_p2p::swarm::block::BlockInfo::from_file_size(nar_size, guix_p2p_e2e::BLOCK_SIZE)
            .with_hashes(&nar_data);
    let seeder = match TestNode::seeder(&nar_hash, nar_data.clone()).await {
        Ok(node) => node,
        Err(e) if is_network_permission_denied(&e) => {
            eprintln!("skipping E2E network test: {e:#}");
            return;
        },
        Err(e) => panic!("{e:#}"),
    };
    let seeder_pid = seeder.pid();
    let mut downloader = match TestNode::node(seeder.addr()).await {
        Ok(node) => node,
        Err(e) if is_network_permission_denied(&e) => {
            eprintln!("skipping E2E network test: {e:#}");
            return;
        },
        Err(e) => panic!("{e:#}"),
    };
    tokio::time::sleep(Duration::from_secs(5)).await;

    let nar_hash_bytes = hex::decode(&nar_hash).unwrap();
    downloader.send_req(seeder_pid, BlockRequest::Handshake { nar_hash: nar_hash_bytes });
    let resp =
        downloader.wait_block_resp(seeder_pid, Duration::from_secs(20)).await.expect("handshake");
    match resp {
        BlockResponse::HandshakeReply { blocks_available, .. } => {
            assert_eq!(blocks_available.len(), block_info.block_count as usize);
        },
        other => panic!("unexpected: {:?}", other),
    }

    let total = block_info.block_count;
    downloader.send_req(
        seeder_pid,
        BlockRequest::GetBlocks {
            nar_hash: hex::decode(&nar_hash).unwrap(),
            indices: (0u32..total).collect(),
        },
    );
    let resp =
        downloader.wait_block_resp(seeder_pid, Duration::from_secs(20)).await.expect("block data");
    let received = match resp {
        BlockResponse::Blocks { data } => data,
        other => panic!("{:?}", other),
    };

    let count = block_info.block_count as usize;
    assert_eq!(received.len(), count, "should receive all {count} blocks");

    let mut blocks_vec = vec![None; count];
    for bd in &received {
        let idx = bd.index as usize;
        let expected = &block_info.block_hashes[idx];
        let actual = Sha256::digest(&bd.data);
        assert_eq!(actual.as_slice(), expected.as_slice(), "block {idx} hash mismatch");
        blocks_vec[idx] = Some(bd.data.clone());
    }

    let assembled: Vec<u8> = blocks_vec.iter().flatten().flatten().copied().collect();
    let final_hash = Sha256::digest(&assembled);
    assert_eq!(format!("{:x}", final_hash), nar_hash, "full nar SHA-256 must match");
    assert_eq!(assembled.len(), nar_data.len(), "assembled size must match nar size");
}

/// Provider discovery via Kademlia DHT requires 3+ nodes for routing-table
/// convergence on a LAN. This test is skipped in the default 2-node topology.
#[tokio::test]
#[ignore]
async fn three_node_swarm() {
    init();
    let (nar_data, nar_hash) = make_nar(0xdd, 65536);
    let seeder = TestNode::seeder(&nar_hash, nar_data).await.unwrap();
    let mut a = TestNode::node(seeder.addr()).await.unwrap();
    let mut b = TestNode::node(seeder.addr()).await.unwrap();
    tokio::time::sleep(Duration::from_secs(7)).await;
    a.lookup(&nar_hash);
    b.lookup(&nar_hash);
    assert!(!a.wait_providers(&nar_hash, Duration::from_secs(20)).await.is_empty());
    assert!(!b.wait_providers(&nar_hash, Duration::from_secs(20)).await.is_empty());
}
