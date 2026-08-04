//! Shared SSH transport for gateway communication.
//!
//! `ssh_connect()` spawns an SSH subprocess in `--ssh-stdio` mode and returns
//! a split reader/writer pair. Both the desktop and TUI clients use this;
//! the higher-level event mapping stays in each client crate.

use anyhow::{Context, Result, anyhow};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use super::protocol::frames::ClientFrame;
use super::protocol::{ServerFrame, WireFrame, deserialize_wire_frame, serialize_wire_frame};

// ── SshReader / SshWriter (split halves) ──────────────────────────────────

/// How long to wait for the gateway's first frame before concluding the
/// connection is dead.  Spawning `ssh` succeeds even when the host is
/// unreachable or the gateway isn't running, so the first frame (`Hello`,
/// or `AuthChallenge` on TOTP-protected gateways) is the earliest proof
/// that a live gateway is on the other end.
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Read half of an SSH gateway transport.
///
/// Owns the child's stdout and stderr. Designed to be moved into a dedicated
/// reader task that calls `recv_wire()` in a loop.
pub struct SshReader {
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    /// Frame consumed by [`Self::wait_first_frame`], handed back by the
    /// next `recv_wire()` call so the handshake doesn't eat it.
    peeked: Option<WireFrame<ServerFrame>>,
}

impl SshReader {
    /// Receive the next wire frame (length-prefixed bincode) from the gateway.
    ///
    /// Returns `Ok(None)` when the connection is closed (EOF).
    pub async fn recv_wire(&mut self) -> Result<Option<WireFrame<ServerFrame>>> {
        if let Some(frame) = self.peeked.take() {
            return Ok(Some(frame));
        }
        self.recv_wire_inner().await
    }

    async fn recv_wire_inner(&mut self) -> Result<Option<WireFrame<ServerFrame>>> {
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

    /// Wait for the gateway's first frame, verifying that a live gateway is
    /// actually on the other end of the transport.
    ///
    /// Spawning the `ssh` subprocess succeeds regardless of whether the
    /// remote host is reachable, auth works, or the gateway binary exists —
    /// those failures surface later inside the subprocess.  Callers should
    /// invoke this before reporting a connection as established.  The frame
    /// is buffered and delivered by the next `recv_wire()` call.
    ///
    /// Errors distinguish the three failure shapes:
    /// - ssh exited (EOF): the most useful stderr line (e.g. "Connection
    ///   refused", "Permission denied") becomes the error message
    /// - protocol error: passed through
    /// - no response within `timeout`: unreachable host / hung connection
    pub async fn wait_first_frame(&mut self, timeout: std::time::Duration) -> Result<()> {
        if self.peeked.is_some() {
            return Ok(());
        }
        match tokio::time::timeout(timeout, self.recv_wire_inner()).await {
            Ok(Ok(Some(frame))) => {
                self.peeked = Some(frame);
                Ok(())
            }
            Ok(Ok(None)) => {
                let stderr = self.drain_stderr().await;
                Err(anyhow!("{}", parse_ssh_error(&stderr)))
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(anyhow!(
                "Gateway did not respond within {}s — host unreachable or gateway not running",
                timeout.as_secs()
            )),
        }
    }

    /// Drain stderr and return any error text.
    pub async fn drain_stderr(&mut self) -> String {
        let mut buf = Vec::new();
        let _ = self.stderr.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).to_string()
    }
}

/// Extract the most useful diagnostic line from ssh stderr output.
///
/// Prefers well-known connection-failure messages; falls back to the last
/// non-empty line, then to a generic message.
pub fn parse_ssh_error(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .find(|line| {
            line.contains("Permission denied")
                || line.contains("Host key verification failed")
                || line.contains("Connection refused")
                || line.contains("Connection timed out")
                || line.contains("No route to host")
                || line.contains("Could not resolve hostname")
                || line.contains("kex_exchange_identification")
        })
        .map(str::to_string)
        .or_else(|| {
            stderr
                .lines()
                .map(str::trim)
                .rfind(|line| !line.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "SSH connection closed".to_string())
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
        self.stdin.flush().await.context("Failed to flush stdin")?;
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
        let parsed =
            url::Url::parse(url).map_err(|e| anyhow!("Invalid SSH URL '{}': {}", url, e))?;

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
        // Reap the ssh subprocess when the connection is dropped (e.g. a
        // failed handshake) instead of leaking it.
        cmd.kill_on_drop(true);

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
            SshReader {
                stdout,
                stderr,
                peeked: None,
            },
        ))
    }

    /// Split an already-spawned child into transport halves.
    ///
    /// Test-only. Lets a test drive the client's reader and writer tasks
    /// against real pipes — including a half-open one, where the write side
    /// is broken while the read side is still healthy — without needing an
    /// SSH host or a gateway.
    #[cfg(test)]
    pub(crate) fn from_child(
        mut child: tokio::process::Child,
    ) -> Result<(Self, SshWriter, SshReader)> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("child stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("child stdout unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("child stderr unavailable"))?;
        Ok((
            Self { child },
            SshWriter { stdin },
            SshReader {
                stdout,
                stderr,
                peeked: None,
            },
        ))
    }

    /// Wait for the child process to exit.
    pub async fn wait(mut self) -> Result<std::process::ExitStatus> {
        self.child.wait().await.context("Failed to wait for SSH")
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Fallback: deserialize a bare `ServerFrame` wrapped in a control wire frame.
fn bare_to_wire_frame(
    data: &[u8],
) -> std::result::Result<WireFrame<ServerFrame>, crate::gateway::protocol::FrameCodecError> {
    // Strict: the whole buffer must be a bare frame, or this is not one.
    //
    // This runs only after the wire-frame decode has already failed, so a
    // lenient decode here answers "is there any frame at the front of these
    // bytes" — and for a control-stream wire frame the answer is always yes,
    // because its header reads as `Hello`/`Empty` in two bytes. That turned
    // every genuine decode failure into a plausible frame the client then
    // discarded without a word, instead of the error it was.
    let frame: ServerFrame = crate::gateway::protocol::deserialize_frame(data)?;
    Ok(WireFrame::control(frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The legacy fallback must not swallow a real wire frame.
    ///
    /// It runs only when the wire decode has already failed, so whatever it
    /// accepts is what the client believes arrived. A control-stream wire
    /// frame starts `[version=3, stream=0, …]`, and those two bytes read as a
    /// complete bare `ServerFrame` — `Hello` with an `Empty` payload — which
    /// `from_server_frame` maps to no event at all. Every undecodable frame
    /// therefore became a frame that silently was not there: no error, no log,
    /// a healthy connection, and a view that never filled in. That is what
    /// hid an undeliverable transcript for as long as it did.
    #[test]
    fn the_bare_fallback_rejects_a_wire_frame() {
        use crate::gateway::protocol::types::ChatMessage;
        use crate::gateway::{ServerFrameType, ServerPayload, serialize_wire_frame};

        let wire = WireFrame::control(ServerFrame {
            frame_type: ServerFrameType::ThreadHistoryReply,
            payload: ServerPayload::ThreadHistoryReply {
                thread_id: 2,
                ok: true,
                messages: vec![ChatMessage::text("user", "hello there")],
                error: None,
            },
        });
        let bytes = serialize_wire_frame(&wire).expect("a wire frame encodes");

        match bare_to_wire_frame(&bytes) {
            Ok(decoded) => panic!(
                "the fallback claimed a wire frame was a bare {:?}, discarding the payload",
                decoded.frame.frame_type
            ),
            Err(e) => {
                // Rejected because it did not account for the whole buffer —
                // the specific check that closes this hole.
                assert!(
                    matches!(
                        e,
                        crate::gateway::protocol::FrameCodecError::TrailingBytes { .. }
                    ),
                    "expected a trailing-bytes rejection, got: {e}"
                );
            }
        }
    }

    /// A genuine bare frame still decodes, so old gateways keep working.
    ///
    /// The strictness above must reject partial reads without rejecting the
    /// case the fallback exists for.
    #[test]
    fn the_bare_fallback_still_accepts_a_bare_frame() {
        use crate::gateway::{ServerFrameType, ServerPayload, serialize_frame};

        let bare = ServerFrame {
            frame_type: ServerFrameType::Status,
            payload: ServerPayload::Empty,
        };
        let bytes = serialize_frame(&bare).expect("a bare frame encodes");
        let decoded = bare_to_wire_frame(&bytes).expect("a bare frame is still accepted");
        assert_eq!(decoded.frame.frame_type, ServerFrameType::Status);
    }

    #[test]
    fn parse_ssh_error_prefers_known_diagnostics() {
        let stderr = "Warning: Permanently added 'host' to the list of known hosts.\n\
                      ssh: connect to host 10.0.0.99 port 2222: Connection refused\n";
        assert_eq!(
            parse_ssh_error(stderr),
            "ssh: connect to host 10.0.0.99 port 2222: Connection refused"
        );

        let stderr = "banner line\nuser@host: Permission denied (publickey).\n";
        assert_eq!(
            parse_ssh_error(stderr),
            "user@host: Permission denied (publickey)."
        );
    }

    #[test]
    fn parse_ssh_error_falls_back_to_last_line_then_generic() {
        assert_eq!(
            parse_ssh_error("something unusual happened\n"),
            "something unusual happened"
        );
        assert_eq!(parse_ssh_error("  \n\n"), "SSH connection closed");
        assert_eq!(parse_ssh_error(""), "SSH connection closed");
    }
}
