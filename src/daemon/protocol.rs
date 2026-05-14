use std::io::{self, BufRead};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

pub enum DaemonCommand {
    Have(Vec<String>),
    Info(Vec<String>),
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

pub fn parse_command_line(line: &str) -> io::Result<DaemonCommand> {
    match line {
        line if let Some(rest) = line.strip_prefix("have ") => {
            Ok(DaemonCommand::Have(rest.split_whitespace().map(str::to_string).collect()))
        },
        line if let Some(rest) = line.strip_prefix("info ") => {
            Ok(DaemonCommand::Info(rest.split_whitespace().map(str::to_string).collect()))
        },
        line if let Some(rest) = line.strip_prefix("substitute ") => {
            let mut parts = rest.splitn(2, ' ');
            let path = parts.next().unwrap_or("").to_string();
            let dest = parts.next().unwrap_or("").to_string();
            Ok(DaemonCommand::Substitute { path, dest })
        },
        path => Ok(DaemonCommand::Have(vec![path.to_string()])),
    }
}

pub enum ReplyWriter {
    /// Write structured replies to fd 4 and trace messages to stdout.
    Fd4,
    /// Collect all output into a buffer (no channel prefix, for direct use).
    Buffer(Vec<u8>),
    /// Collect output with channel prefix framing for socket relay.
    /// fd4: lines -> fd 4, out: lines -> stdout, nar: lines -> restored substitute.
    Socket { buf: Vec<u8> },
}

impl ReplyWriter {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        ReplyWriter::Fd4
    }

    pub fn buffer() -> Self {
        ReplyWriter::Buffer(Vec::new())
    }

    pub fn socket() -> Self {
        ReplyWriter::Socket { buf: Vec::new() }
    }

    /// Write a structured reply line.
    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        match self {
            ReplyWriter::Fd4 => write_fd(4, line),
            ReplyWriter::Buffer(buf) => write_buffered_line(buf, line.as_bytes(), None),
            ReplyWriter::Socket { buf } => write_buffered_line(buf, line.as_bytes(), Some(b"fd4:")),
        }
    }

    /// Write a trace output line.
    pub fn write_trace(&mut self, line: &str) -> io::Result<()> {
        match self {
            ReplyWriter::Fd4 => write_fd(1, line),
            ReplyWriter::Buffer(buf) => write_buffered_line(buf, line.as_bytes(), None),
            ReplyWriter::Socket { buf } => write_buffered_line(buf, line.as_bytes(), Some(b"out:")),
        }
    }

    /// Send raw NAR bytes to a socket relay.
    pub fn write_nar_data(&mut self, nar_data: &[u8]) -> io::Result<()> {
        const CHUNK_SIZE: usize = 48 * 1024;

        if let ReplyWriter::Socket { buf } = self {
            for chunk in nar_data.chunks(CHUNK_SIZE) {
                buf.extend_from_slice(b"nar:");
                buf.extend_from_slice(BASE64.encode(chunk).as_bytes());
                buf.push(b'\n');
            }
            buf.extend_from_slice(b"nar-end\n");
        }

        Ok(())
    }

    pub fn is_socket(&self) -> bool {
        matches!(self, ReplyWriter::Socket { .. })
    }

    pub fn write_end(&mut self) -> io::Result<()> {
        self.write_line("")
    }

    pub async fn flush_socket(
        &mut self,
        writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    ) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        match self {
            ReplyWriter::Socket { buf } if !buf.is_empty() => {
                writer.write_all(buf).await?;
                buf.clear();
                writer.flush().await
            },
            _ => Ok(()),
        }
    }

    pub fn has_data(&self) -> bool {
        match self {
            ReplyWriter::Buffer(buf) | ReplyWriter::Socket { buf } => !buf.is_empty(),
            ReplyWriter::Fd4 => false,
        }
    }

    pub fn into_buffer(self) -> Option<Vec<u8>> {
        match self {
            ReplyWriter::Buffer(buf) => Some(buf),
            _ => None,
        }
    }
}

fn write_fd(fd: libc::c_int, line: &str) -> io::Result<()> {
    let mut buf = line.as_bytes().to_vec();
    buf.push(b'\n');
    unsafe {
        let n = libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len());
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn write_buffered_line(buf: &mut Vec<u8>, line: &[u8], prefix: Option<&[u8]>) -> io::Result<()> {
    if let Some(prefix) = prefix {
        buf.extend_from_slice(prefix);
    }
    buf.extend_from_slice(line);
    buf.push(b'\n');
    Ok(())
}

/// An `std::io::Write` implementation that collects lines into a Vec<String>.
pub struct LineBuffer {
    lines: Vec<String>,
}

impl Default for LineBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl LineBuffer {
    pub fn new() -> Self {
        LineBuffer { lines: Vec::new() }
    }

    pub fn into_string(self) -> String {
        self.lines.iter().map(|line| format!("{line}\n")).collect()
    }
}

impl io::Write for LineBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.lines.extend(String::from_utf8_lossy(buf).lines().map(str::to_string));
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn format_trace_started(store_path: &str, url: &str, size: u64) -> String {
    format!("@ download-started {store_path} {url} {size}")
}

pub fn format_trace_progress(store_path: &str, url: &str, total: u64, transferred: u64) -> String {
    format!("@ download-progress {store_path} {url} {total} {transferred}")
}

pub fn format_trace_succeeded(store_path: &str, url: &str, size: u64) -> String {
    format!("@ download-succeeded {store_path} {url} {size}")
}
