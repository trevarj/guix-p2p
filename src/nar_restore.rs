use anyhow::Context;

/// Write a NAR to the destination path requested by Guix.
///
/// Guix's substitute protocol asks the substituter to materialize the
/// downloaded archive at `DESTINATION`.  The daemon restores that archive into
/// the store after the substituter reports success on fd 4.
pub async fn write_nar_to_destination(
    nar_path: &std::path::Path,
    dest: &std::path::Path,
) -> anyhow::Result<()> {
    match tokio::fs::symlink_metadata(dest).await {
        Ok(metadata) if metadata.is_dir() => {
            tokio::fs::remove_dir_all(dest).await.with_context(|| {
                format!("failed to remove existing directory {}", dest.display())
            })?;
        },
        Ok(_) => {
            tokio::fs::remove_file(dest)
                .await
                .with_context(|| format!("failed to remove existing file {}", dest.display()))?;
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
        Err(e) => {
            return Err(e).with_context(|| format!("failed to inspect {}", dest.display()));
        },
    }

    tokio::fs::copy(nar_path, dest).await.with_context(|| {
        format!("failed to write nar {} to {}", nar_path.display(), dest.display())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_nar_to_destination;

    #[tokio::test]
    async fn writes_nar_bytes_to_destination() {
        let root = std::env::temp_dir().join(format!(
            "guix-p2p-nar-dest-test-{}-{}",
            std::process::id(),
            "writes"
        ));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.unwrap();

        let nar = root.join("source.nar");
        let dest = root.join("dest.nar");
        tokio::fs::write(&nar, b"nar-bytes").await.unwrap();

        write_nar_to_destination(&nar, &dest).await.unwrap();

        let data = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(data, b"nar-bytes");
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn replaces_existing_destination_directory() {
        let root = std::env::temp_dir().join(format!(
            "guix-p2p-nar-dest-test-{}-{}",
            std::process::id(),
            "replaces"
        ));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.unwrap();

        let nar = root.join("source.nar");
        let dest = root.join("dest");
        tokio::fs::write(&nar, b"nar-bytes").await.unwrap();
        tokio::fs::create_dir(&dest).await.unwrap();
        tokio::fs::write(dest.join("old"), b"old").await.unwrap();

        write_nar_to_destination(&nar, &dest).await.unwrap();

        let data = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(data, b"nar-bytes");
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
