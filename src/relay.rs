use anyhow::Context;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

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

    // Forward stdin → socket in a background task
    let copy_task =
        tokio::spawn(
            async move { tokio::io::copy(&mut tokio::io::stdin(), &mut socket_write).await },
        );

    // Read socket replies → demux to fd 4 or stdout
    let mut reader = tokio::io::BufReader::new(socket_read);
    let mut line = String::new();

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

    let _ = copy_task.await;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub enum RelayMode {
    Query,
    Substitute,
}
