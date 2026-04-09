use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

pub const SOCKET_PATH: &str = "/tmp/ten-four.sock";

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Toggle,
    Status,
}

impl Command {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Command::Toggle => b"toggle",
            Command::Status => b"status",
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        match bytes {
            b"toggle" => Some(Command::Toggle),
            b"status" => Some(Command::Status),
            _ => None,
        }
    }
}

/// Send a command to the daemon and return the response string.
pub async fn send_command(socket_path: &str, cmd: Command) -> Result<String> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!(
            "Could not connect to ten-four daemon at {}. Is the daemon running?",
            socket_path
        ))?;

    stream.write_all(cmd.as_bytes()).await?;
    stream.shutdown().await?;

    let mut response = String::new();
    stream.read_to_string(&mut response).await?;

    Ok(response.trim().to_string())
}

/// Bind and return a UnixListener, removing any stale socket file first.
pub fn bind_socket(socket_path: &str) -> Result<UnixListener> {
    // Remove stale socket if it exists
    if std::path::Path::new(socket_path).exists() {
        std::fs::remove_file(socket_path)
            .with_context(|| format!("Failed to remove stale socket at {}", socket_path))?;
    }

    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("Failed to bind Unix socket at {}", socket_path))?;

    Ok(listener)
}

/// Accept a single connection, read the command, call handler, write response.
pub async fn accept_one(
    listener: &UnixListener,
    handler: impl Fn(Command) -> String,
) -> Result<()> {
    let (mut stream, _) = listener.accept().await?;

    let mut buf = vec![0u8; 64];
    let n = stream.read(&mut buf).await?;
    buf.truncate(n);

    let response = match Command::from_bytes(&buf) {
        Some(cmd) => handler(cmd),
        None => "unknown command".to_string(),
    };

    stream.write_all(response.as_bytes()).await?;
    Ok(())
}
