//! Shared SSH transport for gateway communication.
//!
//! `ssh_connect()` spawns an SSH subprocess in `--ssh-stdio` mode and returns
//! a split reader/writer pair. Both the desktop and TUI clients use this;
//! the higher-level event mapping stays in each client crate.

use anyhow::{Context, Result, anyhow};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use super::protocol::{ServerFrame, WireFrame, deserialize_wire_frame, serialize_wire_frame};
use super::protocol::frames::ClientFrame;

// ── SshReader / SshWriter (split halves) ──────────────────────────────────

/// Read half of an SSH gateway transport.
///
/// Owns the child's stdout and stderr. Designed to be moved into a dedicated
/// reader task that calls `recv_wire()` in a loop.
pub struct SshReader {
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
}

impl SshReader {
    /// Receive the next wire frame (length-prefixed bincode) from the gateway.
    ///
    /// Returns `Ok(None)` when the connection is closed (EOF).
    pub async fn recv_wire(&mut self) -> Result<Option<WireFrame<ServerFrame>>> {
        let mut len_buf = [0u8; 4];
        match self.stdout.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(anyhow!("SSH read error: {}", e)),
        }

        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 16 * 1024 * 1024 {
            anyhow::bail!("SSH frame too large ({} bytes)", len);
        }

        let mut frame_buf = vec![0u8; len];
        self.stdout
            .read_exact(&mut frame_buf)
            .await
            .context("Failed to read frame body")?;

        // Try wire-frame format first, fall back to bare frame.
        let wire = deserialize_wire_frame::<ServerFrame>(&frame_buf)
            .or_else(|_| bare_to_wire_frame(&frame_buf))
            .map_err(|e| anyhow!("Failed to decode frame: {}", e))?;

        Ok(Some(wire))
    }

    /// Drain stderr and return any error text.
    pub async fn drain_stderr(&mut self) -> String {
        let mut buf = Vec::new();
        let _ = self.stderr.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).to_string()
    }
}

/// Write half of an SSH gateway transport.
///
/// Owns the child's stdin. Designed to be moved into a dedicated writer task.
pub struct SshWriter {
    stdin: tokio::process::ChildStdin,
}

impl SshWriter {
    /// Send a `ClientFrame` as a length-prefixed bincode wire frame.
    pub async fn send_frame(&mut self, stream_id: u64, frame: &ClientFrame) -> Result<()> {
        let wire = WireFrame::new(stream_id, frame.clone());
        let data =
            serialize_wire_frame(&wire).map_err(|e| anyhow!("Failed to serialize frame: {}", e))?;
        self.send_raw(&data).await
    }

    /// Send raw bytes (length-prefixed).
    pub async fn send_raw(&mut self, data: &[u8]) -> Result<()> {
        let len = data.len() as u32;
        self.stdin
            .write_all(&len.to_be_bytes())
            .await
            .context("Failed to write frame length")?;
        self.stdin
            .write_all(data)
            .await
            .context("Failed to write frame data")?;
        self.stdin
            .flush()
            .await
            .context("Failed to flush stdin")?;
        Ok(())
    }
}

// ── SshConnection (owns the child handle) ────────────────────────────────

/// Manages a single SSH gateway subprocess lifecycle.
///
/// ## Usage
///
/// ```ignore
/// let (conn, mut writer, mut reader) = SshConnection::connect("ssh://host").await?;
///
/// // Move halves into separate tasks:
/// tokio::spawn(async move {
///     // ... write side ...
/// });
///
/// // Read side:
/// while let Some(wire) = reader.recv_wire().await? { ... }
/// ```
pub struct SshConnection {
    child: tokio::process::Child,
}

impl SshConnection {
    /// Parse `url` (`ssh://[user@]host[:port]`), spawn an SSH subprocess
    /// running `rustyclaw-gateway run --ssh-stdio`, and return split
    /// reader + writer halves.
    pub async fn connect(url: &str) -> Result<(Self, SshWriter, SshReader)> {
        let parsed = url::Url::parse(url)
            .map_err(|e| anyhow!("Invalid SSH URL '{}': {}", url, e))?;

        if parsed.scheme() != "ssh" {
            anyhow::bail!("Unsupported scheme '{}'; expected ssh://", parsed.scheme());
        }

        let host = parsed.host_str().unwrap_or("localhost").to_string();
        let port = parsed.port();
        let user = if parsed.username().is_empty() {
            None
        } else {
            Some(parsed.username().to_string())
        };

        // Ensure we have a RustyClaw client identity key.
        let client_key_path = crate::pairing::ClientKeyPair::load_or_generate(None)
            .map(|_| crate::pairing::default_client_key_path())
            .context("Failed to load/generate client key")?;

        // ── Build the SSH command ──────────────────────────────────────
        let mut cmd = Command::new("ssh");
        cmd.arg("-T");
        cmd.arg("-o").arg("PreferredAuthentications=publickey");
        cmd.arg("-o").arg("IdentitiesOnly=yes");
        cmd.arg("-i").arg(&client_key_path);

        let known_hosts_path = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("rustyclaw")
            .join("known_hosts");
        cmd.arg("-o")
            .arg(format!("UserKnownHostsFile={}", known_hosts_path.display()));
        cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
        cmd.arg("-o").arg("BatchMode=yes");

        if let Some(p) = port {
            cmd.arg("-p").arg(p.to_string());
        }
        let target = if let Some(u) = &user {
            format!("{}@{}", u, host)
        } else {
            host
        };
        cmd.arg(&target);
        cmd.arg("rustyclaw-gateway").arg("run").arg("--ssh-stdio");

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().context("Failed to spawn ssh")?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("SSH stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("SSH stdout unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("SSH stderr unavailable"))?;

        Ok((
            Self { child },
            SshWriter { stdin },
            SshReader { stdout, stderr },
        ))
    }

    /// Wait for the child process to exit.
    pub async fn wait(mut self) -> Result<std::process::ExitStatus> {
        self.child.wait().await.context("Failed to wait for SSH")
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Fallback: deserialize a bare `ServerFrame` wrapped in a control wire frame.
fn bare_to_wire_frame(data: &[u8]) -> std::result::Result<WireFrame<ServerFrame>, String> {
    let frame: ServerFrame = bincode::serde::decode_from_slice(data, bincode::config::standard())
        .map(|(f, _)| f)
        .map_err(|e| format!("Bincode decode error: {}", e))?;
    Ok(WireFrame::control(frame))
}
