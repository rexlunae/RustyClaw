//! WhatsApp messenger — wa-rs client implementation.
//!
//! This backend uses the unofficial WhatsApp Web protocol via the [`wa-rs`] crate.
//! On first run the library will display a QR code (via [`tracing`]) that must be
//! scanned with the WhatsApp mobile app to authenticate.  Subsequent runs reuse the
//! persisted session stored in the SQLite database at `db_path`.
//!
//! # Disclaimer
//! This is an unofficial client.  Using custom WhatsApp clients may violate Meta's
//! Terms of Service and could result in account suspension.  Use at your own risk.

use crate::{Message, Messenger};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use wa_rs::types::events::Event;
use wa_rs::{bot::Bot, proto_helpers::MessageExt, store::SqliteStore, Client, Jid};
use wa_rs_tokio_transport::TokioWebSocketTransportFactory;
use wa_rs_ureq_http::UreqHttpClient;

/// WhatsApp messenger backed by the `wa-rs` client.
///
/// Create with [`WhatsAppMessenger::new`], then call [`Messenger::initialize`] to connect.
/// On first use a QR code will be emitted through [`tracing`]; scan it with the WhatsApp
/// mobile app.  Subsequent sessions are restored automatically from `db_path`.
pub struct WhatsAppMessenger {
    name: String,
    /// Path to the SQLite file used for session persistence.
    db_path: String,
    client: Option<Arc<Client>>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
    connected: bool,
    messages: Arc<Mutex<Vec<Message>>>,
}

impl WhatsAppMessenger {
    /// Create a new `WhatsAppMessenger`.
    ///
    /// * `name` — logical name used by [`MessengerManager`](crate::MessengerManager).
    /// * `db_path` — path to the SQLite session database (e.g. `"whatsapp.db"`).
    pub fn new(name: impl Into<String>, db_path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            db_path: db_path.into(),
            client: None,
            task_handle: None,
            connected: false,
            messages: Arc::new(Mutex::new(Vec::new())),
        }
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

    /// Connect to WhatsApp.
    ///
    /// Opens (or creates) the SQLite session database at `db_path`, registers an
    /// event handler that queues inbound text messages, and spawns the background
    /// networking task.  The method returns as soon as the task is running; the
    /// actual WhatsApp handshake happens asynchronously.
    ///
    /// If no prior session exists the library will emit a QR code via
    /// `tracing::info!` — scan it with the WhatsApp mobile app to authenticate.
    async fn initialize(&mut self) -> Result<()> {
        let backend = Arc::new(SqliteStore::new(&self.db_path).await?);
        let messages = self.messages.clone();

        let mut bot = Bot::builder()
            .with_backend(backend)
            .with_transport_factory(TokioWebSocketTransportFactory::new())
            .with_http_client(UreqHttpClient::new())
            .on_event(move |event, _client| {
                let messages = messages.clone();
                async move {
                    match event {
                        Event::PairingQrCode { code, .. } => {
                            tracing::info!(
                                "WhatsApp QR code — scan with the WhatsApp mobile app:\n{code}"
                            );
                        }
                        Event::Message(msg, info) => {
                            if let Some(text) = msg.text_content() {
                                let m = Message {
                                    id: info.id.clone(),
                                    sender: info.source.sender.to_string(),
                                    content: text.to_string(),
                                    timestamp: info.timestamp.timestamp(),
                                    channel: Some(info.source.chat.to_string()),
                                    reply_to: None,
                                    media: None,
                                    is_direct: !info.source.is_group,
                                    reactions: None,
                                };
                                if let Ok(mut msgs) = messages.lock() {
                                    msgs.push(m);
                                }
                            }
                        }
                        Event::Connected(_) => {
                            tracing::info!("WhatsApp connected.");
                        }
                        _ => {}
                    }
                }
            })
            .build()
            .await?;

        self.client = Some(bot.client());
        let handle = bot.run().await?;
        self.task_handle = Some(handle);
        self.connected = true;
        Ok(())
    }

    /// Send a text message to `recipient`.
    ///
    /// `recipient` can be a full JID (e.g. `"15551234567@s.whatsapp.net"`) or a
    /// bare phone number (e.g. `"15551234567"`), which is normalised to
    /// `<number>@s.whatsapp.net`.
    async fn send_message(&self, recipient: &str, text: &str) -> Result<String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("WhatsApp messenger is not initialised"))?;

        let jid: Jid = if recipient.contains('@') {
            recipient
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid JID '{recipient}': {e}"))?
        } else {
            format!("{recipient}@s.whatsapp.net")
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid phone number '{recipient}': {e}"))?
        };

        let message = wa_rs::wa_rs_proto::whatsapp::Message {
            conversation: Some(text.to_string()),
            ..Default::default()
        };

        let id = client.send_message(jid, message).await?;
        Ok(id)
    }

    /// Drain and return all inbound text messages received since the last call.
    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let mut msgs = self
            .messages
            .lock()
            .map_err(|e| anyhow::anyhow!("message queue mutex poisoned: {e}"))?;
        Ok(std::mem::take(&mut *msgs))
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        self.client = None;
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
        Ok(())
    }
}
