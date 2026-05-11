use anyhow::Context;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use crate::nar_restore::restore_nar_to_destination;

/// Connect to the daemon's Unix socket, forward stdin bytes to it,
/// read reply lines and route them to the correct file descriptor.
///
/// The socket protocol uses channel prefix framing:
///   `fd4:<line>` → write to fd 4 (structured replies)
///   `out:<line>` → write to fd 1 (trace messages like @ download-started)
///
/// The relay sends a mode header first so the daemon knows which
/// mode the relay is running in:
///   `mode: query` or `mode: substitute`
/// Then it pipes stdin to the socket and socket replies to the correct fds.
pub async fn forward(socket_path: &str, mode: RelayMode) -> anyhow::Result<()> {
    let stream = tokio::net::UnixStream::connect(socket_path)
        .await
        .context("failed to connect to daemon socket")?;

    let (socket_read, mut socket_write) = stream.into_split();

    // Send mode header
    let mode_line = match mode {
        RelayMode::Query => "mode: query\n",
        RelayMode::Substitute => "mode: substitute\n",
    };
    socket_write
        .write_all(mode_line.as_bytes())
        .await
        .context("failed to send mode header to daemon")?;

    let (dest_tx, mut dest_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let copy_task = if matches!(mode, RelayMode::Query) {
        Some(tokio::spawn(async move {
            tokio::io::copy(&mut tokio::io::stdin(), &mut socket_write).await
        }))
    } else {
        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
        let mut stdin_line = String::new();
        let n = stdin
            .read_line(&mut stdin_line)
            .await
            .context("failed to read substitute command from stdin")?;
        if n == 0 {
            return Err(anyhow::anyhow!("stdin closed before substitute command"));
        }

        if let Some(dest) = substitute_destination(stdin_line.trim_end()) {
            let _ = dest_tx.send(dest.to_string());
        }

        socket_write
            .write_all(stdin_line.as_bytes())
            .await
            .context("failed to forward substitute command to daemon socket")?;
        socket_write.shutdown().await.context("failed to close daemon socket write half")?;
        None
    };

    // Read socket replies → demux to fd 4 or stdout
    let mut reader = tokio::io::BufReader::new(socket_read);
    let mut line = String::new();
    let mut nar_dest: Option<String> = None;
    let mut nar_file: Option<tokio::fs::File> = None;
    let mut nar_temp_path: Option<std::path::PathBuf> = None;
    let mut nar_index = 0u64;

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }

        if let Some(data) = line.strip_prefix("fd4:") {
            // Structured reply → fd 4
            let buf = data.as_bytes();
            unsafe {
                let written = libc::write(4, buf.as_ptr() as *const libc::c_void, buf.len());
                if written < 0 {
                    return Err(anyhow::anyhow!(
                        "fd 4 write failed: {}",
                        std::io::Error::last_os_error()
                    ));
                }
            }
        } else if let Some(data) = line.strip_prefix("out:") {
            // Trace output → stdout
            let buf = data.as_bytes();
            unsafe {
                let written = libc::write(1, buf.as_ptr() as *const libc::c_void, buf.len());
                if written < 0 {
                    return Err(anyhow::anyhow!(
                        "stdout write failed: {}",
                        std::io::Error::last_os_error()
                    ));
                }
            }
        } else if let Some(data) = line.strip_prefix("nar:") {
            if nar_file.is_none() {
                let dest = dest_rx
                    .recv()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("received nar data without a destination"))?;
                nar_index += 1;
                let temp_path = std::env::temp_dir()
                    .join(format!("guix-p2p-relay-{}-{nar_index}.nar", std::process::id()));
                let file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .truncate(true)
                    .write(true)
                    .open(&temp_path)
                    .await
                    .with_context(|| {
                        format!("failed to open temporary nar {}", temp_path.display())
                    })?;
                nar_dest = Some(dest);
                nar_temp_path = Some(temp_path);
                nar_file = Some(file);
            }

            let decoded = BASE64
                .decode(data.trim_end())
                .context("failed to decode nar chunk from daemon socket")?;
            if let Some(file) = nar_file.as_mut() {
                file.write_all(&decoded)
                    .await
                    .context("failed to write nar chunk to temporary nar")?;
            }
        } else if line == "nar-end\n" || line == "nar-end\r\n" {
            if let Some(mut file) = nar_file.take() {
                file.flush().await.context("failed to flush temporary nar")?;
            }
            let dest = nar_dest
                .take()
                .ok_or_else(|| anyhow::anyhow!("received nar-end without a destination"))?;
            let temp_path = nar_temp_path
                .take()
                .ok_or_else(|| anyhow::anyhow!("received nar-end without a temporary nar"))?;
            tracing::info!(
                nar = %temp_path.display(),
                dest = %dest,
                "restoring nar to substitute destination"
            );
            restore_nar_to_destination(&temp_path, std::path::Path::new(&dest)).await?;
            tracing::info!(dest = %dest, "restored nar to substitute destination");
            let _ = tokio::fs::remove_file(&temp_path).await;
        } else {
            // Legacy unprefixed line → treat as fd 4 data for backward compat
            tracing::warn!("unprefixed socket line (treating as fd4): {:?}", line.trim());
            let buf = line.as_bytes();
            unsafe {
                let written = libc::write(4, buf.as_ptr() as *const libc::c_void, buf.len());
                if written < 0 {
                    return Err(anyhow::anyhow!(
                        "fd 4 write failed: {}",
                        std::io::Error::last_os_error()
                    ));
                }
            }
        }
    }

    if let Some(dest) = nar_dest {
        if let Some(temp_path) = nar_temp_path {
            let _ = tokio::fs::remove_file(temp_path).await;
        }
        return Err(anyhow::anyhow!("daemon socket closed before finishing nar for {dest}"));
    }

    if let Some(task) = copy_task {
        let _ = task.await;
    }
    Ok(())
}

fn substitute_destination(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("substitute ")?;
    let mut parts = rest.splitn(2, ' ');
    let _store_path = parts.next()?;
    parts.next().filter(|dest| !dest.is_empty())
}

#[derive(Debug, Clone, Copy)]
pub enum RelayMode {
    Query,
    Substitute,
}

#[cfg(test)]
mod tests {
    use super::substitute_destination;

    #[test]
    fn parses_substitute_destination() {
        assert_eq!(
            substitute_destination("substitute /gnu/store/abc-foo /gnu/store/abc-foo"),
            Some("/gnu/store/abc-foo")
        );
        assert_eq!(substitute_destination("have /gnu/store/abc-foo"), None);
    }
}
