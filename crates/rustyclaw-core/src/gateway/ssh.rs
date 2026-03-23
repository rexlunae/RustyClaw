#[cfg(feature = "ssh")]
pub mod ssh_transport {
    use super::*;
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::fs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{mpsc, RwLock};
    
    use russh::server::{self, Auth, Msg, Session};
    use russh::{Channel, ChannelId};
    use russh_keys::key;

    #[derive(Debug, Clone)]
    pub struct SshTransportConfig {
        pub bind_addr: SocketAddr,
        pub host_keys: Vec<PathBuf>,
        pub authorized_keys_file: Option<PathBuf>,
        pub subsystem_name: Option<String>, // If None, uses stdio mode
    }

    impl Default for SshTransportConfig {
        fn default() -> Self {
            Self {
                bind_addr: "0.0.0.0:2222".parse().unwrap(),
                host_keys: vec![],
                authorized_keys_file: None,
                subsystem_name: Some("rustyclaw".to_string()),
            }
        }
    }

    pub struct SshTransport {
        connection_info: ConnectionInfo,
        sender: mpsc::UnboundedSender<TransportMessage>,
        receiver: mpsc::UnboundedReceiver<TransportMessage>,
        connected: Arc<std::sync::atomic::AtomicBool>,
    }

    impl SshTransport {
        fn new(
            connection_info: ConnectionInfo,
        ) -> (Self, mpsc::UnboundedSender<TransportMessage>, mpsc::UnboundedReceiver<TransportMessage>) {
            let (tx_in, rx_in) = mpsc::unbounded_channel();
            let (tx_out, rx_out) = mpsc::unbounded_channel();
            
            let transport = Self {
                connection_info,
                sender: tx_out,
                receiver: rx_in,
                connected: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            };
            
            (transport, tx_in, rx_out)
        }
    }

    #[async_trait]
    impl Transport for SshTransport {
        fn connection_info(&self) -> &ConnectionInfo {
            &self.connection_info
        }

        async fn send(&mut self, message: TransportMessage) -> Result<(), TransportError> {
            if !self.is_connected() {
                return Err(TransportError::ConnectionClosed);
            }
            
            self.sender.send(message).map_err(|_| TransportError::ConnectionClosed)
        }

        async fn receive(&mut self) -> Result<Option<TransportMessage>, TransportError> {
            match self.receiver.recv().await {
                Some(msg) => Ok(Some(msg)),
                None => {
                    self.connected.store(false, std::sync::atomic::Ordering::Relaxed);
                    Ok(None)
                }
            }
        }

        async fn close(&mut self) -> Result<(), TransportError> {
            self.connected.store(false, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        fn is_connected(&self) -> bool {
            self.connected.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    pub struct SshTransportFactory {
        authorized_keys: Arc<RwLock<HashMap<String, String>>>, // fingerprint -> username
    }

    impl SshTransportFactory {
        pub fn new() -> Self {
            Self {
                authorized_keys: Arc::new(RwLock::new(HashMap::new())),
            }
        }

        async fn load_authorized_keys(&self, path: &PathBuf) -> Result<(), TransportError> {
            let content = fs::read_to_string(path).await
                .map_err(|e| TransportError::Other(format!("Failed to read authorized keys: {}", e)))?;
            
            let mut keys = self.authorized_keys.write().await;
            keys.clear();
            
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                
                // Parse SSH public key format: "ssh-rsa AAAAB3... user@host"
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let key_data = format!("{} {}", parts[0], parts[1]);
                    let username = parts.get(2).unwrap_or("unknown").to_string();
                    
                    // Generate fingerprint (simplified)
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    
                    let mut hasher = DefaultHasher::new();
                    key_data.hash(&mut hasher);
                    let fingerprint = format!("{:x}", hasher.finish());
                    
                    keys.insert(fingerprint, username);
                }
            }
            
            Ok(())
        }
    }

    #[async_trait]
    impl TransportFactory for SshTransportFactory {
        type Transport = SshTransport;
        type Config = SshTransportConfig;

        async fn listen(&self, config: Self::Config) -> Result<TransportListener<Self::Transport>, TransportError> {
            // Load authorized keys if specified
            if let Some(auth_keys_path) = &config.authorized_keys_file {
                self.load_authorized_keys(auth_keys_path).await?;
            }

            let (tx, rx) = mpsc::unbounded_channel();
            let factory = Arc::new(self.clone());
            
            let handle = tokio::spawn(async move {
                if let Err(e) = run_ssh_server(config, factory, tx).await {
                    log::error!("SSH server error: {}", e);
                }
            });

            Ok(TransportListener::new(rx, handle))
        }
    }

    impl Clone for SshTransportFactory {
        fn clone(&self) -> Self {
            Self {
                authorized_keys: Arc::clone(&self.authorized_keys),
            }
        }
    }

    async fn run_ssh_server(
        config: SshTransportConfig,
        factory: Arc<SshTransportFactory>,
        connection_sender: mpsc::UnboundedSender<Result<SshTransport, TransportError>>,
    ) -> Result<(), TransportError> {
        let listener = TcpListener::bind(&config.bind_addr).await
            .map_err(|e| TransportError::Other(format!("Failed to bind to {}: {}", config.bind_addr, e)))?;
        
        log::info!("SSH server listening on {}", config.bind_addr);

        while let Ok((stream, addr)) = listener.accept().await {
            let factory_clone = Arc::clone(&factory);
            let config_clone = config.clone();
            let connection_sender_clone = connection_sender.clone();
            
            tokio::spawn(async move {
                match handle_ssh_connection(stream, addr, config_clone, factory_clone).await {
                    Ok(transport) => {
                        if connection_sender_clone.send(Ok(transport)).is_err() {
                            log::warn!("Failed to send new SSH connection to listener");
                        }
                    }
                    Err(e) => {
                        log::warn!("SSH connection from {} failed: {}", addr, e);
                        let _ = connection_sender_clone.send(Err(e));
                    }
                }
            });
        }

        Ok(())
    }

    async fn handle_ssh_connection(
        stream: TcpStream,
        addr: SocketAddr,
        config: SshTransportConfig,
        factory: Arc<SshTransportFactory>,
    ) -> Result<SshTransport, TransportError> {
        // Create SSH server configuration
        let server_config = russh::server::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(3600)),
            auth_rejection_time: std::time::Duration::from_secs(3),
            auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
            keys: vec![], // Host keys would be loaded here
            ..Default::default()
        };

        let server = SshServer::new(factory, config.subsystem_name);
        
        // This is a simplified version - a full implementation would need
        // proper SSH server setup with russh
        let connection_info = ConnectionInfo::Ssh {
            remote_addr: addr.to_string(),
            username: "unknown".to_string(), // Would be filled during auth
            public_key_fingerprint: None,
        };

        let (transport, _tx_in, _rx_out) = SshTransport::new(connection_info);
        
        // TODO: Integrate with actual russh server session handling
        // This is a placeholder structure
        
        Ok(transport)
    }

    struct SshServer {
        factory: Arc<SshTransportFactory>,
        subsystem_name: Option<String>,
        sessions: Arc<RwLock<HashMap<usize, SessionState>>>,
    }

    struct SessionState {
        username: String,
        authenticated: bool,
        transport_tx: Option<mpsc::UnboundedSender<TransportMessage>>,
    }

    impl SshServer {
        fn new(factory: Arc<SshTransportFactory>, subsystem_name: Option<String>) -> Self {
            Self {
                factory,
                subsystem_name,
                sessions: Arc::new(RwLock::new(HashMap::new())),
            }
        }
    }

    #[async_trait]
    impl server::Server for SshServer {
        type Handler = Self;

        fn new_client(&mut self, _peer_addr: Option<SocketAddr>) -> Self {
            Self {
                factory: Arc::clone(&self.factory),
                subsystem_name: self.subsystem_name.clone(),
                sessions: Arc::clone(&self.sessions),
            }
        }
    }

    #[async_trait]
    impl server::Handler for SshServer {
        type Error = TransportError;

        async fn channel_open_session(
            &mut self,
            _channel: Channel<Msg>,
            _session: &mut Session,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }

        async fn auth_publickey(
            &mut self,
            _user: &str,
            _public_key: &key::PublicKey,
        ) -> Result<Auth, Self::Error> {
            // TODO: Implement proper public key authentication
            // Check against authorized_keys
            Ok(Auth::Accept)
        }

        async fn data(
            &mut self,
            _channel: ChannelId,
            data: &[u8],
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            // Handle incoming data from SSH channel
            let message = TransportMessage {
                content: data.to_vec(),
                message_type: MessageType::Binary,
            };
            
            // TODO: Forward to transport sender
            log::debug!("Received {} bytes via SSH", data.len());
            
            Ok(())
        }

        async fn subsystem_request(
            &mut self,
            _channel: ChannelId,
            name: &str,
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            if let Some(expected_name) = &self.subsystem_name {
                if name == expected_name {
                    log::info!("SSH subsystem '{}' requested", name);
                    return Ok(());
                }
            }
            
            Err(TransportError::ProtocolError(format!("Unsupported subsystem: {}", name)))
        }
    }
}

#[cfg(feature = "ssh")]
pub use ssh_transport::*;