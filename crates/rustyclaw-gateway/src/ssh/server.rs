//! SSH server: russh-backed acceptor, auth handler, and transport.
//!
//! Accepts SSH connections (public-key auth against the authorized-clients
//! list, optional password/TOTP), and adapts each channel into the gateway's
//! [`Transport`] abstraction.

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
            let key_data =
                std::fs::read_to_string(&ssh_config.host_key_path).with_context(|| {
                    format!(
                        "Failed to read host key: {}",
                        ssh_config.host_key_path.display()
                    )
                })?;

            // Parse as OpenSSH private key
            russh::keys::PrivateKey::from_openssh(&key_data).with_context(|| {
                format!(
                    "Failed to parse host key: {}",
                    ssh_config.host_key_path.display()
                )
            })?
        } else {
            info!("Generating new SSH host key");

            // Generate a new Ed25519 key
            let key = russh::keys::PrivateKey::random(
                &mut rand_core::OsRng,
                russh::keys::Algorithm::Ed25519,
            )
            .context("Failed to generate host key")?;

            // Ensure parent directory exists
            if let Some(parent) = ssh_config.host_key_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // Save the key in OpenSSH format
            let key_data = key
                .to_openssh(russh::keys::ssh_key::LineEnding::LF)
                .context("Failed to encode host key")?;
            std::fs::write(&ssh_config.host_key_path, key_data.as_bytes()).with_context(|| {
                format!(
                    "Failed to save host key: {}",
                    ssh_config.host_key_path.display()
                )
            })?;

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

        // Build russh server config — publickey auth only.
        // Restricting methods prevents standard SSH clients from being
        // prompted for a password, which would never succeed.
        let config = russh::server::Config {
            keys: vec![host_key],
            methods: russh::MethodSet::from(&[russh::MethodKind::PublicKey][..]),
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
        let authorized_clients_path = self.ssh_config.authorized_clients_path.clone();
        let allow_unknown_keys_with_totp = self.ssh_config.allow_unknown_keys_with_totp;
        let tx = self.connection_tx.clone();

        info!(address = %addr, "SSH server listening");

        // Spawn the server
        tokio::spawn(async move {
            let mut handler = SshHandler {
                authorized_clients: authorized,
                authorized_clients_path,
                allow_unknown_keys_with_totp,
                peer_addr: None,
                authenticated_username: None,
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
        let rx = self
            .connection_rx
            .as_mut()
            .context("SSH server not initialized")?;

        rx.recv()
            .await
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
    channel_data_tx: mpsc::Sender<Vec<u8>>,
}

/// SSH connection handler.
struct SshHandler {
    authorized_clients: Arc<Mutex<Vec<AuthorizedClient>>>,
    authorized_clients_path: PathBuf,
    allow_unknown_keys_with_totp: bool,
    peer_addr: Option<SocketAddr>,
    authenticated_username: Option<String>,
    connection_tx: mpsc::Sender<SshTransport>,
    sessions: Arc<Mutex<HashMap<ChannelId, ClientSession>>>,
}

impl Clone for SshHandler {
    fn clone(&self) -> Self {
        Self {
            authorized_clients: self.authorized_clients.clone(),
            authorized_clients_path: self.authorized_clients_path.clone(),
            allow_unknown_keys_with_totp: self.allow_unknown_keys_with_totp,
            peer_addr: self.peer_addr,
            authenticated_username: self.authenticated_username.clone(),
            connection_tx: self.connection_tx.clone(),
            sessions: self.sessions.clone(),
        }
    }
}

impl Server for SshHandler {
    type Handler = Self;

    fn new_client(&mut self, peer_addr: Option<SocketAddr>) -> Self::Handler {
        let mut cloned = self.clone();
        cloned.peer_addr = peer_addr;
        cloned.authenticated_username = None;
        cloned
    }
}

impl Handler for SshHandler {
    type Error = anyhow::Error;

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Acknowledge PTY allocation so OpenSSH clients don't block
        // waiting for a success/failure reply.
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // We don't provide an interactive shell, but must explicitly
        // acknowledge the request to avoid client-side hangs.
        session.channel_success(channel)?;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // TUI uses an exec-style request (`ssh host rustyclaw-gateway --ssh-stdio`).
        // Accept it and continue using channel data as raw framed transport.
        session.channel_success(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        _name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Accept subsystem requests for compatibility with OpenSSH subsystem mode.
        session.channel_success(channel)?;
        Ok(())
    }

    async fn auth_password(&mut self, user: &str, _password: &str) -> Result<Auth, Self::Error> {
        warn!(user = user, "Password auth attempted but not supported");
        Ok(Auth::Reject {
            proceed_with_methods: Some(russh::MethodSet::from(&[russh::MethodKind::PublicKey][..])),
            partial_success: false,
        })
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        let fingerprint = key_fingerprint(public_key);
        debug!(user = user, fingerprint = %fingerprint, "Public key auth attempt");

        let mut clients = self.authorized_clients.lock().await;

        // Bootstrap mode: first connecting key is persisted and trusted.
        if clients.is_empty() {
            let comment = {
                let c = public_key.comment();
                if c.is_empty() {
                    Some(format!("{}@rustyclaw", user))
                } else {
                    Some(c.to_string())
                }
            };

            add_authorized_client(
                &self.authorized_clients_path,
                public_key,
                comment.as_deref(),
            )?;

            clients.push(AuthorizedClient {
                key: public_key.clone(),
                comment: comment.clone(),
            });

            warn!(
                user = user,
                fingerprint = %fingerprint,
                path = %self.authorized_clients_path.display(),
                "Bootstrapped first SSH client key into authorized_clients"
            );

            self.authenticated_username = Some(user.to_string());

            return Ok(Auth::Accept);
        }

        for client in clients.iter() {
            if &client.key == public_key {
                info!(
                    user = user,
                    fingerprint = %fingerprint,
                    comment = ?client.comment,
                    "SSH key authenticated"
                );
                self.authenticated_username = Some(user.to_string());
                return Ok(Auth::Accept);
            }
        }

        warn!(user = user, fingerprint = %fingerprint, "Unknown SSH key");
        if self.allow_unknown_keys_with_totp {
            warn!(
                user = user,
                fingerprint = %fingerprint,
                "Allowing unknown SSH key because TOTP is enabled"
            );
            self.authenticated_username = Some(user.to_string());
            return Ok(Auth::Accept);
        }
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
        sessions.insert(
            channel.id(),
            ClientSession {
                channel_data_tx: data_tx,
            },
        );

        // Create the transport
        let transport = SshTransport {
            peer_info: PeerInfo {
                addr: self.peer_addr,
                username: self.authenticated_username.clone(),
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
        // Never await while holding the session map lock.
        let tx = {
            let sessions = self.sessions.lock().await;
            sessions.get(&channel).map(|s| s.channel_data_tx.clone())
        };
        if let Some(tx) = tx {
            let _ = tx.send(data.to_vec()).await;
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

    async fn recv(&mut self) -> Result<Option<WireFrame<ClientFrame>>> {
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
                    return decode_client_wire_frame(&frame_data).map(Some);
                }
            }

            // Need more data
            match data_rx.recv().await {
                Some(data) => buffer.extend(data),
                None => return Ok(None),
            }
        }
    }

    async fn send_on_stream(&mut self, stream_id: u64, frame: &ServerFrame) -> Result<()> {
        let data = encode_server_wire_frame(stream_id, frame)?;
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
        let SshTransport {
            peer_info,
            data_rx,
            channel_handle,
            recv_buffer,
        } = *self;

        (
            Box::new(SshReader {
                peer_info: peer_info.clone(),
                data_rx,
                recv_buffer,
            }),
            Box::new(SshWriter { channel_handle }),
        )
    }
}

struct SshReader {
    peer_info: PeerInfo,
    data_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    recv_buffer: Mutex<Vec<u8>>,
}

#[async_trait]
impl TransportReader for SshReader {
    async fn recv(&mut self) -> Result<Option<WireFrame<ClientFrame>>> {
        let mut buffer = self.recv_buffer.lock().await;
        let mut data_rx = self.data_rx.lock().await;

        loop {
            if buffer.len() >= 4 {
                let len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;

                if len > MAX_FRAME_SIZE as usize {
                    anyhow::bail!("Frame too large: {} bytes", len);
                }

                if buffer.len() >= 4 + len {
                    let frame_data: Vec<u8> = buffer.drain(..4 + len).skip(4).collect();
                    return decode_client_wire_frame(&frame_data).map(Some);
                }
            }

            match data_rx.recv().await {
                Some(data) => buffer.extend(data),
                None => return Ok(None),
            }
        }
    }

    fn peer_info(&self) -> &PeerInfo {
        &self.peer_info
    }
}

struct SshWriter {
    channel_handle: Arc<Mutex<Option<Channel<Msg>>>>,
}

#[async_trait]
impl TransportWriter for SshWriter {
    async fn send_on_stream(&mut self, stream_id: u64, frame: &ServerFrame) -> Result<()> {
        let data = encode_server_wire_frame(stream_id, frame)?;
        let len = data.len() as u32;

        let mut packet = Vec::with_capacity(4 + data.len());
        packet.extend_from_slice(&len.to_be_bytes());
        packet.extend_from_slice(&data);

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
}
