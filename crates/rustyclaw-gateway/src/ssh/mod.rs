//! SSH transport implementation using `russh`.
//!
//! This module provides SSH server functionality for the gateway, allowing
//! clients to connect via SSH instead of WebSocket. This enables:
//!
//! - **Native SSH authentication**: Use existing SSH keys instead of TOTP
//! - **Tunneling through firewalls**: SSH is often allowed where WebSocket isn't
//! - **Integration with SSH agents**: Key management via ssh-agent
//! - **OpenSSH subsystem mode**: Run as `rustyclaw-gateway --ssh-stdio` subsystem
//!
//! ## Modes
//!
//! ### Standalone Server Mode
//!
//! The gateway listens on a dedicated SSH port (default 2222):
//!
//! ```bash
//! rustyclaw-gateway --ssh-listen 0.0.0.0:2222
//! ```
//!
//! Clients connect with:
//! ```bash
//! rustyclaw-tui --ssh user@host:2222
//! ```
//!
//! ### OpenSSH Subsystem Mode
//!
//! Add to `~/.ssh/config`:
//! ```text
//! Host myagent
//!   HostName myserver.example.com
//!   User agent
//!   RequestTTY no
//!   RemoteCommand rustyclaw-gateway --ssh-stdio
//! ```
//!
//! Or configure as a proper subsystem in `/etc/ssh/sshd_config`:
//! ```text
//! Subsystem rustyclaw /usr/local/bin/rustyclaw-gateway --ssh-stdio
//! ```
//!
//! ## Authentication
//!
//! SSH connections are authenticated via public key. The gateway maintains
//! an `authorized_clients` file (similar to `authorized_keys`) that lists
//! allowed public keys:
//!
//! ```text
//! # ~/.rustyclaw/authorized_clients
//! ssh-ed25519 AAAAC3NzaC1lZDI1NTE5... user@laptop
//! ssh-rsa AAAAB3NzaC1yc2EAAAA... user@desktop
//! ```
//!
//! When a client connects via SSH with a key in this file, TOTP is bypassed.
//!
//! ## Protocol
//!
//! Once the SSH channel is established, the same bincode-serialized frames
//! are exchanged as with WebSocket. Frames are length-prefixed:
//!
//! ```text
//! [4 bytes: frame length (big-endian u32)][N bytes: bincode frame]
//! ```

use anyhow::Result;
use async_trait::async_trait;
use rustyclaw_core::gateway::protocol::{
    ClientFrame, ServerFrame, WireFrame, deserialize_frame, deserialize_wire_frame,
    serialize_wire_frame,
};
use rustyclaw_core::gateway::transport::TransportAcceptor;
use rustyclaw_core::gateway::transport::{
    PeerInfo, Transport, TransportReader, TransportType, TransportWriter,
};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use anyhow::Context;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};

use std::path::Path;

use russh::keys::PublicKey;
use russh::server::{Auth, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId};

/// Maximum frame size (16 MB should be plenty).
const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

fn decode_client_wire_frame(data: &[u8]) -> Result<WireFrame<ClientFrame>> {
    match deserialize_wire_frame::<ClientFrame>(data) {
        Ok(frame) => Ok(frame),
        Err(wire_err) => deserialize_frame::<ClientFrame>(data)
            .map(WireFrame::control)
            .map_err(|frame_err| {
                anyhow::anyhow!(
                    "wire decode failed: {}; legacy decode failed: {}",
                    wire_err,
                    frame_err
                )
            }),
    }
}

fn encode_server_wire_frame(stream_id: u64, frame: &ServerFrame) -> Result<Vec<u8>> {
    serialize_wire_frame(&WireFrame::new(stream_id, frame.clone())).map_err(|e| anyhow::anyhow!(e))
}

/// Configuration for the SSH transport.
#[derive(Debug, Clone)]
pub struct SshConfig {
    // listen_addr / allow_password / require_pubkey are set by callers for
    // completeness but not yet read back: the bind address comes from the
    // SshServer constructor argument, and auth policy is currently enforced
    // at the russh handler level. Kept for a future config-driven path.
    /// Address to listen on (e.g., "0.0.0.0:2222").
    #[allow(dead_code)]
    pub listen_addr: std::net::SocketAddr,
    /// Path to the server's host key.
    pub host_key_path: PathBuf,
    /// Path to the authorized_clients file.
    pub authorized_clients_path: PathBuf,
    /// Whether to allow password authentication (disabled by default).
    #[allow(dead_code)]
    pub allow_password: bool,
    /// Whether to require public key authentication.
    #[allow(dead_code)]
    pub require_pubkey: bool,
    /// Whether unknown client keys may authenticate at SSH layer when
    /// application-layer TOTP is enabled.
    pub allow_unknown_keys_with_totp: bool,
}

impl Default for SshConfig {
    fn default() -> Self {
        let config_dir = dirs::home_dir()
            .map(|h| h.join(".rustyclaw"))
            .unwrap_or_else(|| PathBuf::from("."));

        Self {
            listen_addr: "0.0.0.0:2222".parse().unwrap(),
            host_key_path: config_dir.join("ssh_host_key"),
            authorized_clients_path: config_dir.join("authorized_clients"),
            allow_password: false,
            require_pubkey: true,
            allow_unknown_keys_with_totp: false,
        }
    }
}

/// An authorized client entry.
#[derive(Debug, Clone)]
pub struct AuthorizedClient {
    /// The public key.
    pub key: PublicKey,
    /// Optional comment (usually user@host).
    pub comment: Option<String>,
}

/// Load authorized clients from a file.
///
/// Format is the same as OpenSSH's authorized_keys:
/// ```text
/// ssh-ed25519 AAAAC3NzaC1lZDI1NTE5... comment
/// ```
pub fn load_authorized_clients(path: &Path) -> Result<Vec<AuthorizedClient>> {
    use std::io::{BufRead, BufReader};

    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open authorized_clients: {}", path.display()))?;

    let reader = BufReader::new(file);
    let mut clients = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("Failed to read line {}", line_num + 1))?;
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse the key using russh's internal ssh_key crate
        match russh::keys::PublicKey::from_openssh(line) {
            Ok(key) => {
                // Extract comment from the key
                let comment = {
                    let c = key.comment();
                    if c.is_empty() {
                        None
                    } else {
                        Some(c.to_string())
                    }
                };
                clients.push(AuthorizedClient { key, comment });
            }
            Err(e) => {
                warn!(
                    line = line_num + 1,
                    error = %e,
                    "Failed to parse key in authorized_clients"
                );
            }
        }
    }

    Ok(clients)
}

/// Add a public key to the authorized_clients file.
pub fn add_authorized_client(path: &Path, key: &PublicKey, comment: Option<&str>) -> Result<()> {
    use std::io::Write;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Failed to open authorized_clients: {}", path.display()))?;

    // Use ssh_key's to_openssh method for proper formatting
    let key_line = if let Some(comment) = comment {
        key.to_openssh()
            .map(|s| format!("{} {}", s.trim(), comment))
            .unwrap_or_else(|_| format!("{:?}", key))
    } else {
        key.to_openssh()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| format!("{:?}", key))
    };

    writeln!(file, "{}", key_line)?;
    Ok(())
}

/// Get the fingerprint of a public key.
pub fn key_fingerprint(key: &PublicKey) -> String {
    // Use russh's internal HashAlg
    key.fingerprint(russh::keys::HashAlg::Sha256).to_string()
}

// ============================================================================
// SSH Server Implementation
// ============================================================================

mod server;

pub use server::*;

// ============================================================================
// Stdio Subsystem Transport
// ============================================================================

/// SSH subsystem transport using stdin/stdout.
///
/// This is used when running as `rustyclaw-gateway --ssh-stdio`, typically
/// invoked via OpenSSH's subsystem mechanism.
pub struct StdioTransport {
    peer_info: PeerInfo,
    stdin: tokio::io::Stdin,
    stdout: tokio::io::Stdout,
    recv_buffer: Vec<u8>,
}

impl StdioTransport {
    /// Create a new stdio transport.
    ///
    /// The `username` is typically passed from the SSH server via an
    /// environment variable (e.g., `SSH_USER`).
    pub fn new(username: Option<String>) -> Self {
        Self {
            peer_info: PeerInfo {
                addr: None,
                username,
                key_fingerprint: std::env::var("SSH_KEY_FINGERPRINT").ok(),
                transport_type: TransportType::SshSubsystem,
            },
            stdin: tokio::io::stdin(),
            stdout: tokio::io::stdout(),
            recv_buffer: Vec::new(),
        }
    }
}

#[async_trait]
impl Transport for StdioTransport {
    fn peer_info(&self) -> &PeerInfo {
        &self.peer_info
    }

    async fn recv(&mut self) -> Result<Option<WireFrame<ClientFrame>>> {
        loop {
            // Check if we have a complete frame in the buffer
            if self.recv_buffer.len() >= 4 {
                let len = u32::from_be_bytes([
                    self.recv_buffer[0],
                    self.recv_buffer[1],
                    self.recv_buffer[2],
                    self.recv_buffer[3],
                ]) as usize;

                if len > MAX_FRAME_SIZE as usize {
                    anyhow::bail!("Frame too large: {} bytes", len);
                }

                if self.recv_buffer.len() >= 4 + len {
                    let frame_data: Vec<u8> = self.recv_buffer.drain(..4 + len).skip(4).collect();
                    return decode_client_wire_frame(&frame_data).map(Some);
                }
            }

            // Need more data
            let mut buf = [0u8; 8192];
            let n = self.stdin.read(&mut buf).await?;
            if n == 0 {
                return Ok(None);
            }
            self.recv_buffer.extend_from_slice(&buf[..n]);
        }
    }

    async fn send_on_stream(&mut self, stream_id: u64, frame: &ServerFrame) -> Result<()> {
        let data = encode_server_wire_frame(stream_id, frame)?;
        let len = data.len() as u32;

        // Send length prefix + data
        self.stdout.write_all(&len.to_be_bytes()).await?;
        self.stdout.write_all(&data).await?;
        self.stdout.flush().await?;

        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.stdout.flush().await?;
        Ok(())
    }

    fn into_split(self: Box<Self>) -> (Box<dyn TransportReader>, Box<dyn TransportWriter>) {
        // Stdio can be split since stdin and stdout are separate
        let peer_info = self.peer_info.clone();
        (
            Box::new(StdioReader {
                stdin: self.stdin,
                recv_buffer: self.recv_buffer,
                peer_info,
            }),
            Box::new(StdioWriter {
                stdout: self.stdout,
            }),
        )
    }
}

struct StdioReader {
    stdin: tokio::io::Stdin,
    recv_buffer: Vec<u8>,
    peer_info: PeerInfo,
}

#[async_trait]
impl TransportReader for StdioReader {
    async fn recv(&mut self) -> Result<Option<WireFrame<ClientFrame>>> {
        loop {
            if self.recv_buffer.len() >= 4 {
                let len = u32::from_be_bytes([
                    self.recv_buffer[0],
                    self.recv_buffer[1],
                    self.recv_buffer[2],
                    self.recv_buffer[3],
                ]) as usize;

                if len > MAX_FRAME_SIZE as usize {
                    anyhow::bail!("Frame too large: {} bytes", len);
                }

                if self.recv_buffer.len() >= 4 + len {
                    let frame_data: Vec<u8> = self.recv_buffer.drain(..4 + len).skip(4).collect();
                    return decode_client_wire_frame(&frame_data).map(Some);
                }
            }

            let mut buf = [0u8; 8192];
            let n = self.stdin.read(&mut buf).await?;
            if n == 0 {
                return Ok(None);
            }
            self.recv_buffer.extend_from_slice(&buf[..n]);
        }
    }

    fn peer_info(&self) -> &PeerInfo {
        &self.peer_info
    }
}

struct StdioWriter {
    stdout: tokio::io::Stdout,
}

#[async_trait]
impl TransportWriter for StdioWriter {
    async fn send_on_stream(&mut self, stream_id: u64, frame: &ServerFrame) -> Result<()> {
        let data = encode_server_wire_frame(stream_id, frame)?;
        let len = data.len() as u32;

        self.stdout.write_all(&len.to_be_bytes()).await?;
        self.stdout.write_all(&data).await?;
        self.stdout.flush().await?;

        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.stdout.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssh_config_default() {
        let config = SshConfig::default();
        assert!(
            config
                .host_key_path
                .to_string_lossy()
                .contains("ssh_host_key")
        );
        assert!(
            config
                .authorized_clients_path
                .to_string_lossy()
                .contains("authorized_clients")
        );
        assert!(!config.allow_password);
        assert!(config.require_pubkey);
        assert!(!config.allow_unknown_keys_with_totp);
    }
}
