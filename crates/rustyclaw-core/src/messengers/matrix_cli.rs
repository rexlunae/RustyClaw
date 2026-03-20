//! Matrix messenger using direct HTTP API calls to homeserver.
//!
//! This implementation uses the Matrix Client-Server API directly via HTTP,
//! avoiding external dependencies like matrix-sdk. It provides basic messaging
//! functionality without E2EE support.
//!
//! This requires the `matrix-cli` feature to be enabled.

use super::{Message, Messenger, SendOptions};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;


/// Matrix API response for login
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LoginResponse {
    access_token: String,
    device_id: String,
    user_id: String,
}

/// Matrix API response for room events
#[derive(Debug, Deserialize)]
struct SyncResponse {
    rooms: Option<RoomsResponse>,
    next_batch: String,
}

#[derive(Debug, Deserialize)]
struct RoomsResponse {
    join: Option<serde_json::Map<String, Value>>,
}

/// Matrix API response for sending messages
#[derive(Debug, Deserialize)]
struct SendResponse {
    event_id: String,
}

/// Matrix room event
#[derive(Debug, Deserialize)]
struct RoomEvent {
    #[serde(rename = "type")]
    event_type: String,
    sender: String,
    content: Value,
    event_id: String,
    origin_server_ts: u64,
}

/// Matrix messenger implementation using HTTP API
pub struct MatrixCliMessenger {
    name: String,
    homeserver_url: String,
    user_id: String,
    password: Option<String>,
    access_token: Option<String>,
    device_id: Option<String>,
    client: Client,
    connected: bool,
    sync_token: Arc<Mutex<Option<String>>>,
}

impl MatrixCliMessenger {
    /// Create a new Matrix CLI messenger with password authentication
    pub fn with_password(
        name: String,
        homeserver_url: String,
        user_id: String,
        password: String,
    ) -> Self {
        Self {
            name,
            homeserver_url: homeserver_url.trim_end_matches('/').to_string(),
            user_id,
            password: Some(password),
            access_token: None,
            device_id: None,
            client: Client::new(),
            connected: false,
            sync_token: Arc::new(Mutex::new(None)),
        }
    }

    /// Create a new Matrix CLI messenger with access token authentication
    pub fn with_token(
        name: String,
        homeserver_url: String,
        user_id: String,
        access_token: String,
        device_id: Option<String>,
    ) -> Self {
        Self {
            name,
            homeserver_url: homeserver_url.trim_end_matches('/').to_string(),
            user_id,
            password: None,
            access_token: Some(access_token),
            device_id,
            client: Client::new(),
            connected: false,
            sync_token: Arc::new(Mutex::new(None)),
        }
    }

    /// Build authorization header
    fn auth_header(&self) -> Result<String> {
        self.access_token
            .as_ref()
            .map(|token| format!("Bearer {}", token))
            .ok_or_else(|| anyhow::anyhow!("No access token available"))
    }

    /// Login with password and get access token
    async fn login(&mut self) -> Result<()> {
        let password = self.password.as_ref()
            .context("No password provided for login")?;

        let login_request = json!({
            "type": "m.login.password",
            "user": self.user_id,
            "password": password,
            "initial_device_display_name": "RustyClaw Matrix CLI"
        });

        let url = format!("{}/_matrix/client/v3/login", self.homeserver_url);
        
        let response = self.client
            .post(&url)
            .json(&login_request)
            .send()
            .await
            .context("Failed to send login request")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Login failed: {} - {}", status, error_text);
        }

        let login_response: LoginResponse = response
            .json()
            .await
            .context("Failed to parse login response")?;

        self.access_token = Some(login_response.access_token);
        self.device_id = Some(login_response.device_id);
        
        Ok(())
    }

    /// Resolve room ID from alias or return as-is if already an ID
    async fn resolve_room_id(&self, room_id_or_alias: &str) -> Result<String> {
        // If it looks like a room ID, return as-is
        if room_id_or_alias.starts_with('!') {
            return Ok(room_id_or_alias.to_string());
        }

        // If it's an alias, resolve it
        if room_id_or_alias.starts_with('#') {
            let encoded_alias = urlencoding::encode(room_id_or_alias);
            let url = format!("{}/_matrix/client/v3/directory/room/{}", 
                            self.homeserver_url, encoded_alias);
            
            let response = self.client
                .get(&url)
                .header("Authorization", self.auth_header()?)
                .send()
                .await
                .context("Failed to resolve room alias")?;

            let status = response.status();
            if !status.is_success() {
                anyhow::bail!("Failed to resolve room alias: {}", status);
            }

            let room_info: Value = response.json().await?;
            return room_info["room_id"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow::anyhow!("Room ID not found in response"));
        }

        // Assume it's a room ID if it doesn't match alias pattern
        Ok(room_id_or_alias.to_string())
    }

    /// Perform a sync to get new messages
    async fn sync(&self, timeout_ms: Option<u64>) -> Result<Vec<Message>> {
        let mut url = format!("{}/_matrix/client/v3/sync", self.homeserver_url);
        
        let mut params = Vec::new();
        {
            let token = self.sync_token.lock().await;
            if let Some(ref t) = *token {
                params.push(format!("since={}", t));
            }
        }
        if let Some(timeout) = timeout_ms {
            params.push(format!("timeout={}", timeout));
        }
        
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

        let response = self.client
            .get(&url)
            .header("Authorization", self.auth_header()?)
            .send()
            .await
            .context("Failed to sync")?;

        if !response.status().is_success() {
            anyhow::bail!("Sync failed: {}", response.status());
        }

        let sync_response: SyncResponse = response.json().await?;
        
        // Update sync token
        {
            let mut token = self.sync_token.lock().await;
            *token = Some(sync_response.next_batch.clone());
        }

        let mut messages = Vec::new();

        if let Some(ref rooms) = sync_response.rooms {
            if let Some(ref joined) = rooms.join {
            }
        } else {
        }
        
        if let Some(rooms) = sync_response.rooms {
            if let Some(joined_rooms) = rooms.join {
                for (room_id, room_data) in joined_rooms {
                    if let Some(timeline) = room_data.get("timeline") {
                        if let Some(events) = timeline.get("events") {
                            if let Some(events_array) = events.as_array() {
                                for event_value in events_array {
                                    if let Ok(event) = serde_json::from_value::<RoomEvent>(event_value.clone()) {
                                        if event.event_type == "m.room.message" {
                                            if let Some(body) = event.content.get("body") {
                                                if let Some(body_str) = body.as_str() {
                                                    messages.push(Message {
                                                        id: event.event_id,
                                                        sender: event.sender,
                                                        content: body_str.to_string(),
                                                        timestamp: (event.origin_server_ts / 1000) as i64,
                                                        channel: Some(room_id.clone()),
                                                        reply_to: None,
                                                        media: None,
                                                    });
                                                }
                                            }
                                        }
                                    } else {
                                    }
                                }
                            } else {
                            }
                        } else {
                        }
                    } else {
                    }
                }
            }
        }

        Ok(messages)
    }

    /// Send a plain text message to a room
    async fn send_text_message(&self, room_id: &str, content: &str, reply_to: Option<&str>) -> Result<String> {
        let resolved_room_id = self.resolve_room_id(room_id).await?;
        
        let txn_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();

        let mut message_content = json!({
            "msgtype": "m.text",
            "body": content
        });

        // Handle reply-to if provided
        if let Some(reply_event_id) = reply_to {
            let reply_body = format!("> <@{}> {}\n\n{}", 
                                   reply_event_id, 
                                   "Previous message", // We'd need to fetch the original to show proper content
                                   content);
            message_content["body"] = json!(reply_body);
            message_content["m.relates_to"] = json!({
                "m.in_reply_to": {
                    "event_id": reply_event_id
                }
            });
        }

        let url = format!("{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}", 
                         self.homeserver_url, 
                         urlencoding::encode(&resolved_room_id),
                         txn_id);

        let response = self.client
            .put(&url)
            .header("Authorization", self.auth_header()?)
            .json(&message_content)
            .send()
            .await
            .context("Failed to send message")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to send message: {} - {}", status, error_text);
        }

        let send_response: SendResponse = response.json().await?;
        Ok(send_response.event_id)
    }
}

#[async_trait]
impl Messenger for MatrixCliMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "matrix-cli"
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.access_token.is_none() && self.password.is_some() {
            self.login().await?;
        }

        if self.access_token.is_none() {
            anyhow::bail!("No access token available and no password provided");
        }

        // Test the connection by doing an initial sync
        self.sync(Some(0)).await?;
        
        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        self.send_text_message(recipient, content, None).await
    }

    async fn send_message_with_options(&self, opts: SendOptions<'_>) -> Result<String> {
        self.send_text_message(opts.recipient, opts.content, opts.reply_to).await
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        // sync() now takes &self and uses Arc<Mutex> for sync_token
        let result = self.sync(Some(1000)).await;
        result
    }

    fn is_connected(&self) -> bool {
        self.connected && self.access_token.is_some()
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(access_token) = &self.access_token {
            let url = format!("{}/_matrix/client/v3/logout", self.homeserver_url);
            
            let _ = self.client
                .post(&url)
                .header("Authorization", format!("Bearer {}", access_token))
                .send()
                .await;
        }

        self.access_token = None;
        self.device_id = None;
        self.connected = false;
        {
            let mut token = self.sync_token.lock().await;
            *token = None;
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matrix_cli_messenger_creation() {
        let messenger = MatrixCliMessenger::with_password(
            "test".to_string(),
            "https://matrix.org".to_string(),
            "@test:matrix.org".to_string(),
            "password".to_string(),
        );
        assert_eq!(messenger.name(), "test");
        assert_eq!(messenger.messenger_type(), "matrix-cli");
        assert!(!messenger.is_connected());
    }

    #[test]
    fn test_matrix_cli_messenger_with_token() {
        let messenger = MatrixCliMessenger::with_token(
            "test".to_string(),
            "https://matrix.org".to_string(),
            "@test:matrix.org".to_string(),
            "syt_token".to_string(),
            Some("DEVICEID".to_string()),
        );
        assert_eq!(messenger.name(), "test");
        assert_eq!(messenger.messenger_type(), "matrix-cli");
        assert!(!messenger.is_connected());
    }

    #[test]
    fn test_homeserver_url_trimming() {
        let messenger = MatrixCliMessenger::with_password(
            "test".to_string(),
            "https://matrix.org/".to_string(),
            "@test:matrix.org".to_string(),
            "password".to_string(),
        );
        assert_eq!(messenger.homeserver_url, "https://matrix.org");
    }
}