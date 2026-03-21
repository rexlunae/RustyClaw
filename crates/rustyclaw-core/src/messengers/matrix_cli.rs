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
use pulldown_cmark::{html, Parser};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
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
    invite: Option<serde_json::Map<String, Value>>,
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

/// DM configuration for Matrix messenger
#[derive(Debug, Clone, Default)]
pub struct MatrixDmConfig {
    /// Whether DMs are enabled
    pub enabled: bool,
    /// DM policy: "allowlist", "open", or "pairing"
    pub policy: String,
    /// List of user IDs allowed to send DMs (for allowlist policy)
    pub allow_from: Vec<String>,
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
    /// Configured allowed chat room IDs (explicit allowlist)
    allowed_chats: HashSet<String>,
    /// DM configuration
    dm_config: MatrixDmConfig,
    /// Dynamically accepted DM room IDs (from auto-accept)
    dm_rooms: Arc<Mutex<HashSet<String>>>,
    /// Directory for persisting state (sync token, etc.)
    state_dir: Option<std::path::PathBuf>,
    /// Messages from initial sync, waiting to be returned
    pending_messages: Arc<Mutex<Vec<Message>>>,
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
            allowed_chats: HashSet::new(),
            dm_config: MatrixDmConfig::default(),
            dm_rooms: Arc::new(Mutex::new(HashSet::new())),
            state_dir: None,
            pending_messages: Arc::new(Mutex::new(Vec::new())),
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
            allowed_chats: HashSet::new(),
            dm_config: MatrixDmConfig::default(),
            dm_rooms: Arc::new(Mutex::new(HashSet::new())),
            state_dir: None,
            pending_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Set state directory for persisting sync token
    pub fn with_state_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.state_dir = Some(dir);
        self
    }

    /// Set allowed chat room IDs
    pub fn with_allowed_chats(mut self, chats: Vec<String>) -> Self {
        self.allowed_chats = chats.into_iter().collect();
        self
    }

    /// Set DM configuration
    pub fn with_dm_config(mut self, config: MatrixDmConfig) -> Self {
        self.dm_config = config;
        self
    }

    /// Build authorization header
    fn auth_header(&self) -> Result<String> {
        self.access_token
            .as_ref()
            .map(|token| format!("Bearer {}", token))
            .ok_or_else(|| anyhow::anyhow!("No access token available"))
    }

    /// Load sync token from disk if state_dir is configured
    fn load_sync_token(&self) -> Option<String> {
        let state_dir = self.state_dir.as_ref()?;
        let token_path = state_dir.join("matrix_sync_token");
        std::fs::read_to_string(&token_path).ok()
    }

    /// Save sync token to disk if state_dir is configured
    fn save_sync_token(&self, token: &str) {
        if let Some(ref state_dir) = self.state_dir {
            let token_path = state_dir.join("matrix_sync_token");
            if let Err(e) = std::fs::create_dir_all(state_dir) {
                eprintln!("Failed to create state dir: {}", e);
                return;
            }
            if let Err(e) = std::fs::write(&token_path, token) {
                eprintln!("Failed to save sync token: {}", e);
            }
        }
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

    /// Join a room by ID
    async fn join_room(&self, room_id: &str) -> Result<()> {
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/join",
            self.homeserver_url,
            urlencoding::encode(room_id)
        );

        let response = self.client
            .post(&url)
            .header("Authorization", self.auth_header()?)
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .context("Failed to join room")?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to join room: {}", error);
        }

        Ok(())
    }

    /// Get list of joined rooms
    async fn get_joined_rooms(&self) -> Result<Vec<String>> {
        let url = format!("{}/_matrix/client/v3/joined_rooms", self.homeserver_url);

        let response = self.client
            .get(&url)
            .header("Authorization", self.auth_header()?)
            .send()
            .await
            .context("Failed to get joined rooms")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to get joined rooms: {}", response.status());
        }

        #[derive(Deserialize)]
        struct JoinedRoomsResponse {
            joined_rooms: Vec<String>,
        }

        let resp: JoinedRoomsResponse = response.json().await?;
        Ok(resp.joined_rooms)
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
        
        // Store next_batch for later - we'll only save it after successful message extraction
        let next_batch = sync_response.next_batch.clone();

        // Process invites if DM is enabled
        if self.dm_config.enabled {
            if let Some(ref rooms) = sync_response.rooms {
                if let Some(ref invites) = rooms.invite {
                    for (room_id, invite_data) in invites {
                        // Find who sent the invite
                        if let Some(invite_state) = invite_data.get("invite_state") {
                            if let Some(events) = invite_state.get("events") {
                                if let Some(events_array) = events.as_array() {
                                    for event in events_array {
                                        if event.get("type").and_then(|t| t.as_str()) == Some("m.room.member") {
                                            if let Some(sender) = event.get("sender").and_then(|s| s.as_str()) {
                                                // Check if we should auto-accept
                                                let should_accept = match self.dm_config.policy.as_str() {
                                                    "open" => true,
                                                    "allowlist" => self.dm_config.allow_from.iter().any(|u| u == sender),
                                                    _ => false,
                                                };
                                                
                                                if should_accept {
                                                    // Accept the invite
                                                    if let Err(e) = self.join_room(room_id).await {
                                                        eprintln!("Failed to auto-accept invite from {}: {}", sender, e);
                                                    } else {
                                                        // Track as DM room
                                                        let mut dm_rooms = self.dm_rooms.lock().await;
                                                        dm_rooms.insert(room_id.clone());
                                                        eprintln!("Auto-accepted DM invite from {} to room {}", sender, room_id);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut messages = Vec::new();

        // Get current DM rooms for filtering
        let dm_rooms = self.dm_rooms.lock().await.clone();
        eprintln!("DEBUG: sync - allowed_chats: {:?}, dm_rooms: {:?}", self.allowed_chats, dm_rooms);
        
        if let Some(rooms) = sync_response.rooms {
            if let Some(joined_rooms) = rooms.join {
                eprintln!("DEBUG: sync - checking {} joined rooms", joined_rooms.len());
                for (room_id, room_data) in joined_rooms {
                    // Check if this room is allowed
                    let in_allowed_chats = self.allowed_chats.contains(&room_id);
                    let in_dm_rooms = dm_rooms.contains(&room_id);
                    
                    // If we have an allowlist OR dm_rooms, only process rooms in one of them
                    if !self.allowed_chats.is_empty() || !dm_rooms.is_empty() {
                        if !in_allowed_chats && !in_dm_rooms {
                            eprintln!("DEBUG: skipping room {} (not in allowed lists)", room_id);
                            continue;
                        }
                    }
                    eprintln!("DEBUG: processing room {}", room_id);
                    
                    if let Some(timeline) = room_data.get("timeline") {
                        if let Some(events) = timeline.get("events") {
                            if let Some(events_array) = events.as_array() {
                                for event_value in events_array {
                                    if let Ok(event) = serde_json::from_value::<RoomEvent>(event_value.clone()) {
                                        // Skip our own messages
                                        if event.sender == self.user_id {
                                            continue;
                                        }
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
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Only update sync token AFTER successful message extraction.
        // This prevents losing messages if extraction fails partway through.
        {
            let mut token = self.sync_token.lock().await;
            *token = Some(next_batch.clone());
            self.save_sync_token(&next_batch);
        }

        Ok(messages)
    }

    /// Set typing indicator for a room
    async fn set_typing(&self, room_id: &str, typing: bool) -> Result<()> {
        let resolved_room_id = self.resolve_room_id(room_id).await?;
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/typing/{}",
            self.homeserver_url,
            urlencoding::encode(&resolved_room_id),
            urlencoding::encode(&self.user_id)
        );

        let body = if typing {
            json!({ "typing": true, "timeout": 30000 })
        } else {
            json!({ "typing": false })
        };

        let response = self.client
            .put(&url)
            .header("Authorization", self.auth_header()?)
            .json(&body)
            .send()
            .await
            .context("Failed to set typing indicator")?;

        if !response.status().is_success() {
            // Non-fatal - just log and continue
            eprintln!("Failed to set typing indicator: {}", response.status());
        }

        Ok(())
    }

    /// Send a plain text message to a room
    async fn send_text_message(&self, room_id: &str, content: &str, reply_to: Option<&str>) -> Result<String> {
        let resolved_room_id = self.resolve_room_id(room_id).await?;
        
        let txn_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();

        // Convert markdown to HTML for formatted display
        let parser = Parser::new(content);
        let mut html_output = String::new();
        html::push_html(&mut html_output, parser);

        let mut message_content = json!({
            "msgtype": "m.text",
            "body": content,
            "format": "org.matrix.custom.html",
            "formatted_body": html_output
        });

        // Handle reply-to if provided
        if let Some(reply_event_id) = reply_to {
            // Note: Modern Matrix clients use m.relates_to for threading and ignore the
            // fallback body. We don't include the legacy "> <@user> text" fallback since
            // we don't have the original message content readily available, and the
            // malformed fallback causes visual garbage in some clients.
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

        // If DM is enabled, load existing joined rooms that aren't in allowed_chats
        // These are likely DM rooms from previous sessions
        if self.dm_config.enabled {
            if let Ok(joined) = self.get_joined_rooms().await {
                let mut dm_rooms = self.dm_rooms.lock().await;
                for room_id in joined {
                    if !self.allowed_chats.contains(&room_id) {
                        dm_rooms.insert(room_id);
                    }
                }
                if !dm_rooms.is_empty() {
                    eprintln!("Loaded {} existing DM rooms", dm_rooms.len());
                }
            }
        }

        // Note: We intentionally do NOT load persisted sync token on restart.
        // Starting fresh ensures we get recent messages (last ~10 per room).
        // The slight risk of re-processing messages on restart is acceptable.
        // The persisted token is saved for potential future use (e.g., quick restart).

        // Do initial sync and store any messages for first receive_messages() call
        let initial_messages = self.sync(Some(0)).await?;
        if !initial_messages.is_empty() {
            eprintln!("Initial sync returned {} messages", initial_messages.len());
            let mut pending = self.pending_messages.lock().await;
            pending.extend(initial_messages);
        }
        
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
        // First, return any pending messages from initial sync
        {
            let mut pending = self.pending_messages.lock().await;
            if !pending.is_empty() {
                let messages = std::mem::take(&mut *pending);
                return Ok(messages);
            }
        }
        
        // Then do normal sync for new messages
        self.sync(Some(1000)).await
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

    async fn set_typing(&self, channel: &str, typing: bool) -> Result<()> {
        MatrixCliMessenger::set_typing(self, channel, typing).await
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
    
    // Note: Own-message filtering (sender == user_id) is tested implicitly
    // via integration tests. The sync() function skips messages where
    // event.sender == self.user_id to prevent the bot from replying to itself.
}