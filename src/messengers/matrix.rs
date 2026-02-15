//! Matrix messenger using matrix-sdk with E2EE support.
//!
//! This requires the `matrix` feature to be enabled.

use super::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use matrix_sdk::{
    config::SyncSettings,
    ruma::{
        api::client::room::create_room::v3::Request as CreateRoomRequest,
        events::room::message::{
            MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
        },
        OwnedRoomId, OwnedUserId, RoomId, UserId,
    },
    Client, Room,
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Matrix messenger with E2EE support
pub struct MatrixMessenger {
    name: String,
    homeserver_url: String,
    user_id: String,
    password: Option<String>,
    access_token: Option<String>,
    device_id: Option<String>,
    store_path: PathBuf,
    client: Option<Client>,
    connected: bool,
    /// Pending incoming messages (populated by sync)
    pending_messages: Arc<Mutex<Vec<Message>>>,
}

impl MatrixMessenger {
    /// Create a new Matrix messenger with password authentication
    pub fn with_password(
        name: String,
        homeserver_url: String,
        user_id: String,
        password: String,
        store_path: PathBuf,
    ) -> Self {
        Self {
            name,
            homeserver_url,
            user_id,
            password: Some(password),
            access_token: None,
            device_id: None,
            store_path,
            client: None,
            connected: false,
            pending_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Create a new Matrix messenger with access token authentication
    pub fn with_token(
        name: String,
        homeserver_url: String,
        user_id: String,
        access_token: String,
        device_id: Option<String>,
        store_path: PathBuf,
    ) -> Self {
        Self {
            name,
            homeserver_url,
            user_id,
            password: None,
            access_token: Some(access_token),
            device_id,
            store_path,
            client: None,
            connected: false,
            pending_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Get the Matrix client (must be initialized first)
    fn client(&self) -> Result<&Client> {
        self.client
            .as_ref()
            .context("Matrix client not initialized")
    }

    /// Resolve a room by ID or alias
    async fn resolve_room(&self, room_id_or_alias: &str) -> Result<Room> {
        let client = self.client()?;

        // Try as room ID first
        if let Ok(room_id) = <&RoomId>::try_from(room_id_or_alias) {
            if let Some(room) = client.get_room(room_id) {
                return Ok(room);
            }
        }

        // Try as room alias
        if room_id_or_alias.starts_with('#') {
            let alias = matrix_sdk::ruma::OwnedRoomAliasId::try_from(room_id_or_alias)
                .context("Invalid room alias")?;
            let response = client.resolve_room_alias(&alias).await?;
            if let Some(room) = client.get_room(&response.room_id) {
                return Ok(room);
            }
        }

        anyhow::bail!("Room not found: {}", room_id_or_alias)
    }
}

#[async_trait]
impl Messenger for MatrixMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "matrix"
    }

    async fn initialize(&mut self) -> Result<()> {
        // Build the client with SQLite store for E2EE state
        let client = Client::builder()
            .homeserver_url(&self.homeserver_url)
            .sqlite_store(&self.store_path, None)
            .build()
            .await
            .context("Failed to build Matrix client")?;

        // Authenticate
        if let Some(ref password) = self.password {
            // Password login
            let user_id = <&UserId>::try_from(self.user_id.as_str())
                .context("Invalid user ID")?;
            
            client
                .matrix_auth()
                .login_username(user_id, password)
                .initial_device_display_name("RustyClaw")
                .send()
                .await
                .context("Matrix login failed")?;
        } else if let Some(ref token) = self.access_token {
            // Token-based session restore
            let session = matrix_sdk::authentication::AuthSession::Matrix(
                matrix_sdk::authentication::matrix::MatrixSession {
                    meta: matrix_sdk::SessionMeta {
                        user_id: OwnedUserId::try_from(self.user_id.as_str())?,
                        device_id: self.device_id
                            .as_ref()
                            .map(|d| matrix_sdk::ruma::OwnedDeviceId::try_from(d.as_str()))
                            .transpose()?
                            .unwrap_or_else(|| "RUSTYCLAW".into()),
                    },
                    tokens: matrix_sdk::authentication::matrix::MatrixSessionTokens {
                        access_token: token.clone(),
                        refresh_token: None,
                    },
                }
            );
            client.restore_session(session).await?;
        } else {
            anyhow::bail!("No authentication method provided");
        }

        // Set up message handler
        let pending = self.pending_messages.clone();
        client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, room: Room| {
            let pending = pending.clone();
            async move {
                let content = match ev.content.msgtype {
                    MessageType::Text(text) => text.body,
                    MessageType::Notice(notice) => notice.body,
                    _ => return,
                };

                let message = Message {
                    id: ev.event_id.to_string(),
                    sender: ev.sender.to_string(),
                    content,
                    timestamp: ev.origin_server_ts.as_secs().into(),
                    channel: Some(room.room_id().to_string()),
                    reply_to: None,
                    media: None,
                };

                pending.lock().await.push(message);
            }
        });

        // Initial sync to catch up
        client.sync_once(SyncSettings::default()).await?;

        self.client = Some(client);
        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, room_id: &str, content: &str) -> Result<String> {
        let room = self.resolve_room(room_id).await?;
        
        let content = RoomMessageEventContent::text_plain(content);
        let response = room.send(content).await?;
        
        Ok(response.event_id.to_string())
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        let room = self.resolve_room(opts.recipient).await?;

        let mut content = RoomMessageEventContent::text_plain(opts.content);

        // Handle reply
        if let Some(reply_to) = opts.reply_to {
            if let Ok(event_id) = matrix_sdk::ruma::OwnedEventId::try_from(reply_to) {
                // For proper threading, we'd need to fetch the original event
                // For now, just reference it in the body
                let reply_body = format!("> Replying to {}\n\n{}", reply_to, opts.content);
                content = RoomMessageEventContent::text_plain(reply_body);
            }
        }

        let response = room.send(content).await?;
        Ok(response.event_id.to_string())
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        let client = self.client()?;

        // Do a quick sync to get new messages
        let settings = SyncSettings::default().timeout(std::time::Duration::from_secs(0));
        let _ = client.sync_once(settings).await;

        // Drain pending messages
        let mut pending = self.pending_messages.lock().await;
        Ok(std::mem::take(&mut *pending))
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(client) = self.client.take() {
            // Logout invalidates the access token
            let _ = client.matrix_auth().logout().await;
        }
        self.connected = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matrix_messenger_creation() {
        let messenger = MatrixMessenger::with_password(
            "test".to_string(),
            "https://matrix.org".to_string(),
            "@test:matrix.org".to_string(),
            "password".to_string(),
            PathBuf::from("/tmp/matrix-test"),
        );
        assert_eq!(messenger.name(), "test");
        assert_eq!(messenger.messenger_type(), "matrix");
        assert!(!messenger.is_connected());
    }
}
