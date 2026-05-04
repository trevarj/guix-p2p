use std::path::Path;

use anyhow::Context;
use libp2p::identity::Keypair;

pub fn load_or_generate_keypair(cache_dir: &Path) -> anyhow::Result<Keypair> {
    std::fs::create_dir_all(cache_dir).context("failed to create cache directory")?;

    let keypair_path = cache_dir.join("keypair");

    if keypair_path.exists() {
        let bytes = std::fs::read(&keypair_path).context("failed to read keypair file")?;
        let keypair = Keypair::from_protobuf_encoding(&bytes)
            .context("failed to decode keypair from protobuf")?;
        tracing::info!("Loaded identity from {}", keypair_path.display());
        Ok(keypair)
    } else {
        let keypair = Keypair::generate_ed25519();
        let bytes = keypair
            .to_protobuf_encoding()
            .map_err(|e| anyhow::anyhow!("failed to encode keypair: {}", e))?;
        std::fs::write(&keypair_path, &bytes).context("failed to write keypair file")?;
        tracing::info!("Generated new identity, saved to {}", keypair_path.display());
        Ok(keypair)
    }
}

pub fn peer_id_from_keypair(kp: &Keypair) -> libp2p::PeerId {
    libp2p::PeerId::from(kp.public())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_load() {
        let dir = tempfile::TempDir::new().unwrap();
        let kp1 = load_or_generate_keypair(dir.path()).unwrap();
        let kp2 = load_or_generate_keypair(dir.path()).unwrap();
        assert_eq!(peer_id_from_keypair(&kp1), peer_id_from_keypair(&kp2));
    }
}
