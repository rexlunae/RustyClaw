//! WhatsApp messenger using wa-rs library.
//!
//! This requires the `whatsapp` feature to be enabled.
//!
//! WhatsApp requires QR code linking before use.
//! Use `WhatsAppMessenger::link_device()` to generate a QR code for linking.

use super::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use wa_rs::Jid;
use wa_rs::bot::Bot;
use wa_rs::client::Client;
use wa_rs::traits::DeviceStore;
use wa_rs::types::events::Event;
use wa_rs::wa_rs_proto::whatsapp as wa;
use wa_rs_sqlite_storage::SqliteStore;
use wa_rs_tokio_transport::TokioWebSocketTransportFactory;
use wa_rs_ureq_http::UreqHttpClient;

/// WhatsApp messenger using wa-rs (Baileys-compatible)
pub struct WhatsAppMessenger {
    name: String,
    db_path: PathBuf,
    client: Option<Arc<Client>>,
    /// Handle for the background bot task
    _bot_handle: Option<JoinHandle<()>>,
    connected: bool,
    /// Pending incoming messages
    pending_messages: Arc<Mutex<Vec<Message>>>,
}

impl WhatsAppMessenger {
    /// Create a new WhatsApp messenger
    ///
    /// The db_path should be a file path for the SQLite database.
    /// If the database already contains session data, it will be used.
    /// Otherwise, you must call `link_device()` before use.
    pub fn new(name: String, db_path: PathBuf) -> Self {
        Self {
            name,
            db_path,
            client: None,
            _bot_handle: None,
            connected: false,
            pending_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Create SQLite backend
    async fn create_backend(db_url: &str) -> Result<Arc<SqliteStore>> {
        SqliteStore::new(db_url)
            .await
            .map(Arc::new)
            .map_err(|e| anyhow::anyhow!("Failed to create WhatsApp backend: {}", e))
    }

    /// Build a bot with our standard configuration
    async fn build_bot(
        backend: Arc<SqliteStore>,
        pending: Arc<Mutex<Vec<Message>>>,
        qr_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<Bot> {
        let builder = Bot::builder()
            .with_backend(backend)
            .with_transport_factory(TokioWebSocketTransportFactory::new())
            .with_http_client(UreqHttpClient::new())
            .on_event(move |event, _client| {
                let pending = pending.clone();
                let qr_tx = qr_tx.clone();
                async move {
                    match &event {
                        Event::PairingQrCode { code, .. } => {
                            if let Some(tx) = qr_tx {
                                let _ = tx.send(code.clone());
                            }
                        }
                        Event::Message(msg, info) => {
                            let content = extract_text_content(msg);
                            if content.is_empty() {
                                return;
                            }

                            let message = Message {
                                id: info.id.clone(),
                                sender: info.source.sender.to_string(),
                                content,
                                timestamp: info.timestamp.timestamp(),
                                channel: Some(info.source.chat.to_string()),
                                reply_to: None,
                                media: None,
                            };

                            pending.lock().await.push(message);
                        }
                        Event::Connected(_) => {
                            tracing::info!("WhatsApp connected");
                        }
                        Event::Disconnected(reason) => {
                            tracing::warn!("WhatsApp disconnected: {:?}", reason);
                        }
                        _ => {}
                    }
                }
            });

        builder
            .build()
            .await
            .context("Failed to build WhatsApp bot")
    }

    /// Link device by scanning a QR code from WhatsApp mobile.
    ///
    /// Returns a receiver that will receive QR code strings to display.
    /// Once scanning is complete, the session will be stored.
    pub async fn link_device(&mut self) -> Result<tokio::sync::mpsc::UnboundedReceiver<String>> {
        let pending = self.pending_messages.clone();
        let db_url = self.db_path.to_string_lossy().to_string();
        let backend = Self::create_backend(&db_url).await?;

        let (qr_tx, qr_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut bot = Self::build_bot(backend, pending, Some(qr_tx)).await?;

        let client = bot.client();
        let handle = bot.run().await.context("Failed to start WhatsApp")?;

        self.client = Some(client);
        self._bot_handle = Some(handle);
        self.connected = true;

        Ok(qr_rx)
    }

    /// Check if the device is linked
    pub fn is_linked(&self) -> bool {
        self.client.is_some()
    }

    /// Get the client (must be linked first)
    fn client(&self) -> Result<&Arc<Client>> {
        self.client
            .as_ref()
            .context("WhatsApp not linked - call link_device() first")
    }

    /// Parse a recipient JID from string
    ///
    /// Formats:
    /// - Phone number: "15551234567" or "+15551234567"
    /// - User JID: "15551234567@s.whatsapp.net"
    /// - Group JID: "123456789@g.us"
    fn parse_jid(recipient: &str) -> Result<Jid> {
        use std::str::FromStr;

        // If already looks like a JID
        if recipient.contains('@') {
            return Jid::from_str(recipient)
                .map_err(|e| anyhow::anyhow!("Invalid JID '{}': {}", recipient, e));
        }

        // Strip + prefix if present
        let number = recipient.trim_start_matches('+');

        // Create user JID (pn = phone number)
        Ok(Jid::pn(number))
    }
}

/// Extract text content from a WhatsApp protobuf Message
fn extract_text_content(msg: &wa::Message) -> String {
    // Check conversation field (simple text message)
    if let Some(ref text) = msg.conversation {
        return text.clone();
    }
    // Check extendedTextMessage field
    if let Some(ref ext) = msg.extended_text_message {
        if let Some(ref text) = ext.text {
            return text.clone();
        }
    }
    String::new()
}

/// Build a simple text message
fn build_text_message(text: &str) -> wa::Message {
    wa::Message {
        conversation: Some(text.to_string()),
        ..Default::default()
    }
}

#[async_trait]
impl Messenger for WhatsAppMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "whatsapp"
    }

    async fn initialize(&mut self) -> Result<()> {
        // Check if we have an existing session
        if !self.db_path.exists() {
            self.connected = false;
            return Ok(());
        }

        let pending = self.pending_messages.clone();
        let db_url = self.db_path.to_string_lossy().to_string();
        let backend = Self::create_backend(&db_url).await?;

        // Check if we have valid session data
        if !backend.exists().await.unwrap_or(false) {
            self.connected = false;
            return Ok(());
        }

        let mut bot = Self::build_bot(backend, pending, None).await?;

        let client = bot.client();
        let handle = bot.run().await.context("Failed to start WhatsApp")?;

        self.client = Some(client);
        self._bot_handle = Some(handle);
        self.connected = true;

        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        let client = self.client()?;
        let jid = Self::parse_jid(recipient)?;

        let message = build_text_message(content);
        let message_id = client
            .send_message(jid, message)
            .await
            .context("Failed to send WhatsApp message")?;

        Ok(message_id)
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        // For now, just send plain text
        // TODO: Add reply_to and media support
        self.send_message(opts.recipient, opts.content).await
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let mut pending = self.pending_messages.lock().await;
        let messages = std::mem::take(&mut *pending);
        Ok(messages)
    }

    fn is_connected(&self) -> bool {
        self.connected && self.client.is_some()
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(client) = self.client.take() {
            client.disconnect().await;
        }
        self._bot_handle = None;
        self.connected = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_whatsapp_messenger_creation() {
        let messenger =
            WhatsAppMessenger::new("test".to_string(), PathBuf::from("/tmp/whatsapp-test.db"));
        assert_eq!(messenger.name(), "test");
        assert_eq!(messenger.messenger_type(), "whatsapp");
        assert!(!messenger.is_connected());
        assert!(!messenger.is_linked());
    }

    #[test]
    fn test_parse_jid() {
        let jid = WhatsAppMessenger::parse_jid("15551234567").unwrap();
        assert!(jid.to_string().contains("15551234567"));

        let jid = WhatsAppMessenger::parse_jid("+15551234567").unwrap();
        assert!(jid.to_string().contains("15551234567"));
    }
}
