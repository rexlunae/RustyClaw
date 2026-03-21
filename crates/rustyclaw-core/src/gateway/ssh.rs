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

use super::protocol::{ClientFrame, ServerFrame, deserialize_frame, serialize_frame};
use super::transport::{PeerInfo, Transport, TransportAcceptor, TransportReader, TransportType, TransportWriter};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};

#[cfg(feature = "ssh")]
use russh::server::{Auth, Handler, Msg, Server, Session};
#[cfg(feature = "ssh")]
use russh::{Channel, ChannelId, CryptoVec};
#[cfg(feature = "ssh")]
use russh::keys::PublicKey;

/// Maximum frame size (16 MB should be plenty).
const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

/// Configuration for the SSH transport.
#[derive(Debug, Clone)]
pub struct SshConfig {
    /// Path to the server's host key.
    pub host_key_path: PathBuf,
    /// Path to the authorized_clients file.
    pub authorized_clients_path: PathBuf,
    /// Whether to allow password authentication (disabled by default).
    pub allow_password: bool,
    /// Whether to require public key authentication.
    pub require_pubkey: bool,
}

impl Default for SshConfig {
    fn default() -> Self {
        let config_dir = dirs::home_dir()
            .map(|h| h.join(".rustyclaw"))
            .unwrap_or_else(|| PathBuf::from("."));
        
        Self {
            host_key_path: config_dir.join("ssh_host_key"),
            authorized_clients_path: config_dir.join("authorized_clients"),
            allow_password: false,
            require_pubkey: true,
        }
    }
}

/// An authorized client entry.
#[derive(Debug, Clone)]
pub struct AuthorizedClient {
    /// The public key.
    #[cfg(feature = "ssh")]
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
#[cfg(feature = "ssh")]
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
                    if c.is_empty() { None } else { Some(c.to_string()) }
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
#[cfg(feature = "ssh")]
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
#[cfg(feature = "ssh")]
pub fn key_fingerprint(key: &PublicKey) -> String {
    // Use russh's internal HashAlg
    key.fingerprint(russh::keys::HashAlg::Sha256).to_string()
}

// ============================================================================
// SSH Server Implementation
// ============================================================================

#[cfg(feature = "ssh")]
mod server {
    use super::*;
    
    /// SSH server that accepts connections and creates transports.
    pub struct SshServer {
        config: Arc<russh::server::Config>,
        ssh_config: SshConfig,
        authorized_clients: Arc<Mutex<Vec<AuthorizedClient>>>,
        /// Channel for sending accepted connections to the acceptor.
        connection_tx: mpsc::Sender<SshTransport>,
        /// Receiver for accepted connections.
        connection_rx: Option<mpsc::Receiver<SshTransport>>,
    }
    
    impl SshServer {
        /// Create a new SSH server.
        pub async fn new(ssh_config: SshConfig) -> Result<Self> {
            #[allow(unused_imports)]
            use std::io::Read;
            
            // Load or generate host key
            let host_key: russh::keys::PrivateKey = if ssh_config.host_key_path.exists() {
                // Read the key file
                let key_data = std::fs::read_to_string(&ssh_config.host_key_path)
                    .with_context(|| format!(
                        "Failed to read host key: {}",
                        ssh_config.host_key_path.display()
                    ))?;
                
                // Parse as OpenSSH private key
                russh::keys::PrivateKey::from_openssh(&key_data)
                    .with_context(|| format!(
                        "Failed to parse host key: {}",
                        ssh_config.host_key_path.display()
                    ))?
            } else {
                info!("Generating new SSH host key");
                
                // Generate a new Ed25519 key
                let key = russh::keys::PrivateKey::random(
                    &mut rand_core::OsRng,
                    russh::keys::Algorithm::Ed25519
                ).context("Failed to generate host key")?;
                
                // Ensure parent directory exists
                if let Some(parent) = ssh_config.host_key_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                
                // Save the key in OpenSSH format
                let key_data = key.to_openssh(russh::keys::ssh_key::LineEnding::LF)
                    .context("Failed to encode host key")?;
                std::fs::write(&ssh_config.host_key_path, key_data.as_bytes())
                    .with_context(|| format!(
                        "Failed to save host key: {}",
                        ssh_config.host_key_path.display()
                    ))?;
                
                // Set restrictive permissions
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        &ssh_config.host_key_path,
                        std::fs::Permissions::from_mode(0o600),
                    )?;
                }
                
                key
            };
            
            // Build russh server config
            let config = russh::server::Config {
                keys: vec![host_key],
                ..Default::default()
            };
            
            // Load authorized clients
            let authorized_clients = if ssh_config.authorized_clients_path.exists() {
                load_authorized_clients(&ssh_config.authorized_clients_path)?
            } else {
                warn!(
                    path = %ssh_config.authorized_clients_path.display(),
                    "No authorized_clients file found; SSH auth will fail"
                );
                Vec::new()
            };
            
            let (tx, rx) = mpsc::channel(16);
            
            Ok(Self {
                config: Arc::new(config),
                ssh_config,
                authorized_clients: Arc::new(Mutex::new(authorized_clients)),
                connection_tx: tx,
                connection_rx: Some(rx),
            })
        }
        
        /// Start listening for SSH connections.
        pub async fn listen(&mut self, addr: SocketAddr) -> Result<()> {
            let config = self.config.clone();
            let authorized = self.authorized_clients.clone();
            let tx = self.connection_tx.clone();
            
            info!(address = %addr, "SSH server listening");
            
            // Spawn the server
            tokio::spawn(async move {
                let mut handler = SshHandler {
                    authorized_clients: authorized,
                    connection_tx: tx,
                    sessions: Arc::new(Mutex::new(HashMap::new())),
                };
                
                // Use Server trait's run_on_address method
                use russh::server::Server;
                if let Err(e) = handler.run_on_address(config, addr).await {
                    error!(error = %e, "SSH server error");
                }
            });
            
            Ok(())
        }
    }
    
    #[async_trait]
    impl TransportAcceptor for SshServer {
        async fn accept(&mut self) -> Result<Box<dyn Transport>> {
            let rx = self.connection_rx.as_mut()
                .context("SSH server not initialized")?;
            
            rx.recv().await
                .context("SSH server closed")
                .map(|t| Box::new(t) as Box<dyn Transport>)
        }
        
        fn local_addr(&self) -> Result<SocketAddr> {
            // TODO: Track the bound address
            Ok("0.0.0.0:2222".parse()?)
        }
    }
    
    /// Session state for a connected client.
    struct ClientSession {
        username: String,
        key_fingerprint: Option<String>,
        peer_addr: Option<SocketAddr>,
        channel_data_tx: Option<mpsc::Sender<Vec<u8>>>,
    }
    
    /// SSH connection handler.
    struct SshHandler {
        authorized_clients: Arc<Mutex<Vec<AuthorizedClient>>>,
        connection_tx: mpsc::Sender<SshTransport>,
        sessions: Arc<Mutex<HashMap<ChannelId, ClientSession>>>,
    }
    
    impl Clone for SshHandler {
        fn clone(&self) -> Self {
            Self {
                authorized_clients: self.authorized_clients.clone(),
                connection_tx: self.connection_tx.clone(),
                sessions: self.sessions.clone(),
            }
        }
    }
    
    impl Server for SshHandler {
        type Handler = Self;
        
        fn new_client(&mut self, _peer_addr: Option<SocketAddr>) -> Self::Handler {
            self.clone()
        }
    }
    
    impl Handler for SshHandler {
        type Error = anyhow::Error;
        
        async fn auth_publickey(
            &mut self,
            user: &str,
            public_key: &PublicKey,
        ) -> Result<Auth, Self::Error> {
            let fingerprint = key_fingerprint(public_key);
            debug!(user = user, fingerprint = %fingerprint, "Public key auth attempt");
            
            let clients = self.authorized_clients.lock().await;
            for client in clients.iter() {
                if &client.key == public_key {
                    info!(
                        user = user,
                        fingerprint = %fingerprint,
                        comment = ?client.comment,
                        "SSH key authenticated"
                    );
                    return Ok(Auth::Accept);
                }
            }
            
            warn!(user = user, fingerprint = %fingerprint, "Unknown SSH key");
            Ok(Auth::reject())
        }
        
        async fn channel_open_session(
            &mut self,
            channel: Channel<Msg>,
            _session: &mut Session,
        ) -> Result<bool, Self::Error> {
            debug!(channel = ?channel.id(), "Session channel opened");
            
            // Create channels for data transfer
            let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>(64);
            let (_response_tx, _response_rx) = mpsc::channel::<Vec<u8>>(64);
            
            // Store session info
            let mut sessions = self.sessions.lock().await;
            sessions.insert(channel.id(), ClientSession {
                username: "unknown".to_string(), // Set during auth
                key_fingerprint: None,
                peer_addr: None,
                channel_data_tx: Some(data_tx),
            });
            
            // Create the transport
            let transport = SshTransport {
                peer_info: PeerInfo {
                    addr: None,
                    username: Some("unknown".to_string()),
                    key_fingerprint: None,
                    transport_type: TransportType::Ssh,
                },
                data_rx: Mutex::new(data_rx),
                channel_handle: Arc::new(Mutex::new(Some(channel))),
                recv_buffer: Mutex::new(Vec::new()),
            };
            
            // Send to acceptor
            if self.connection_tx.send(transport).await.is_err() {
                warn!("Failed to send transport to acceptor");
                return Ok(false);
            }
            
            Ok(true)
        }
        
        async fn data(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            let sessions = self.sessions.lock().await;
            if let Some(client_session) = sessions.get(&channel) {
                if let Some(tx) = &client_session.channel_data_tx {
                    let _ = tx.send(data.to_vec()).await;
                }
            }
            Ok(())
        }
        
        async fn channel_eof(
            &mut self,
            channel: ChannelId,
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            debug!(channel = ?channel, "Channel EOF");
            let mut sessions = self.sessions.lock().await;
            sessions.remove(&channel);
            Ok(())
        }
    }
    
    /// SSH transport wrapping a russh channel.
    pub struct SshTransport {
        peer_info: PeerInfo,
        data_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
        channel_handle: Arc<Mutex<Option<Channel<Msg>>>>,
        recv_buffer: Mutex<Vec<u8>>,
    }
    
    #[async_trait]
    impl Transport for SshTransport {
        fn peer_info(&self) -> &PeerInfo {
            &self.peer_info
        }
        
        async fn recv(&mut self) -> Result<Option<ClientFrame>> {
            let mut buffer = self.recv_buffer.lock().await;
            let mut data_rx = self.data_rx.lock().await;
            
            loop {
                // Check if we have a complete frame in the buffer
                if buffer.len() >= 4 {
                    let len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
                    
                    if len > MAX_FRAME_SIZE as usize {
                        anyhow::bail!("Frame too large: {} bytes", len);
                    }
                    
                    if buffer.len() >= 4 + len {
                        let frame_data: Vec<u8> = buffer.drain(..4 + len).skip(4).collect();
                        return deserialize_frame(&frame_data)
                            .map(Some)
                            .map_err(|e| anyhow::anyhow!(e));
                    }
                }
                
                // Need more data
                match data_rx.recv().await {
                    Some(data) => buffer.extend(data),
                    None => return Ok(None),
                }
            }
        }
        
        async fn send(&mut self, frame: &ServerFrame) -> Result<()> {
            let data = serialize_frame(frame).map_err(|e| anyhow::anyhow!(e))?;
            let len = data.len() as u32;
            
            // Send length prefix + data
            let mut packet = Vec::with_capacity(4 + data.len());
            packet.extend_from_slice(&len.to_be_bytes());
            packet.extend_from_slice(&data);
            
            // Write to channel
            let channel = self.channel_handle.lock().await;
            if let Some(ch) = channel.as_ref() {
                ch.data(&packet[..]).await?;
            }
            
            Ok(())
        }
        
        async fn close(&mut self) -> Result<()> {
            let mut channel = self.channel_handle.lock().await;
            if let Some(ch) = channel.take() {
                ch.eof().await?;
            }
            Ok(())
        }
        
        fn into_split(self: Box<Self>) -> (Box<dyn TransportReader>, Box<dyn TransportWriter>) {
            // For SSH, we can't easily split the channel, so we use Arc
            let peer_info = self.peer_info.clone();
            let shared = Arc::new(Mutex::new(*self));
            (
                Box::new(SshReader { inner: shared.clone(), peer_info: peer_info.clone() }),
                Box::new(SshWriter { inner: shared, _peer_info: peer_info }),
            )
        }
    }
    
    struct SshReader {
        inner: Arc<Mutex<SshTransport>>,
        peer_info: PeerInfo,
    }
    
    #[async_trait]
    impl TransportReader for SshReader {
        async fn recv(&mut self) -> Result<Option<ClientFrame>> {
            let mut transport = self.inner.lock().await;
            transport.recv().await
        }
        
        fn peer_info(&self) -> &PeerInfo {
            &self.peer_info
        }
    }
    
    struct SshWriter {
        inner: Arc<Mutex<SshTransport>>,
        _peer_info: PeerInfo,
    }
    
    #[async_trait]
    impl TransportWriter for SshWriter {
        async fn send(&mut self, frame: &ServerFrame) -> Result<()> {
            let mut transport = self.inner.lock().await;
            transport.send(frame).await
        }
        
        async fn close(&mut self) -> Result<()> {
            let mut transport = self.inner.lock().await;
            transport.close().await
        }
    }
}

#[cfg(feature = "ssh")]
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

    async fn recv(&mut self) -> Result<Option<ClientFrame>> {
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
                    return deserialize_frame(&frame_data)
                        .map(Some)
                        .map_err(|e| anyhow::anyhow!(e));
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

    async fn send(&mut self, frame: &ServerFrame) -> Result<()> {
        let data = serialize_frame(frame).map_err(|e| anyhow::anyhow!(e))?;
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
    async fn recv(&mut self) -> Result<Option<ClientFrame>> {
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
                    return deserialize_frame(&frame_data)
                        .map(Some)
                        .map_err(|e| anyhow::anyhow!(e));
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
    async fn send(&mut self, frame: &ServerFrame) -> Result<()> {
        let data = serialize_frame(frame).map_err(|e| anyhow::anyhow!(e))?;
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
        assert!(config.host_key_path.to_string_lossy().contains("ssh_host_key"));
        assert!(config.authorized_clients_path.to_string_lossy().contains("authorized_clients"));
        assert!(!config.allow_password);
        assert!(config.require_pubkey);
    }
}
