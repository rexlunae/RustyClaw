//! Matrix messenger backed by the Matrix Client-Server HTTP API.

use crate::{Message, Messenger};
use anyhow::{anyhow, ensure, Context, Result};
use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

pub struct MatrixMessenger {
    name: String,
    homeserver: String,
    username: String,
    password: String,
    client: Client,
    access_token: Option<String>,
    user_id: Option<String>,
    sync_token: Mutex<Option<String>>,
    txn_counter: AtomicU64,
    connected: bool,
}

impl MatrixMessenger {
    pub fn new(
        name: impl Into<String>,
        homeserver: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            homeserver: homeserver.into(),
            username: username.into(),
            password: password.into(),
            client: Client::new(),
            access_token: None,
            user_id: None,
            sync_token: Mutex::new(None),
            txn_counter: AtomicU64::new(1),
            connected: false,
        }
    }

    fn validate_config(&self) -> Result<()> {
        ensure!(
            !self.homeserver.trim().is_empty(),
            "Matrix homeserver must not be empty"
        );
        ensure!(
            !self.username.trim().is_empty(),
            "Matrix username must not be empty"
        );
        ensure!(
            !self.password.trim().is_empty(),
            "Matrix password must not be empty"
        );
        Ok(())
    }

    fn access_token(&self) -> Result<&str> {
        self.access_token
            .as_deref()
            .ok_or_else(|| anyhow!("Matrix messenger is not initialized"))
    }

    fn user_id(&self) -> Result<&str> {
        self.user_id
            .as_deref()
            .ok_or_else(|| anyhow!("Matrix messenger is not initialized"))
    }

    fn url_for_segments(&self, segments: &[&str]) -> Result<Url> {
        let mut url = Url::parse(self.homeserver.trim_end_matches('/'))
            .with_context(|| format!("Invalid Matrix homeserver URL: {}", self.homeserver))?;
        {
            let mut path_segments = url
                .path_segments_mut()
                .map_err(|_| anyhow!("Matrix homeserver URL cannot be a base URL"))?;
            path_segments.extend(segments.iter().copied());
        }
        Ok(url)
    }

    fn client_api_url(&self, path: &[&str]) -> Result<Url> {
        let mut segments = vec!["_matrix", "client", "v3"];
        segments.extend_from_slice(path);
        self.url_for_segments(&segments)
    }

    async fn sync_once(&self) -> Result<Vec<Message>> {
        #[derive(Debug, Deserialize)]
        struct SyncResponse {
            next_batch: String,
            #[serde(default)]
            rooms: SyncRooms,
        }

        #[derive(Debug, Default, Deserialize)]
        struct SyncRooms {
            #[serde(default)]
            join: HashMap<String, JoinedRoom>,
        }

        #[derive(Debug, Default, Deserialize)]
        struct JoinedRoom {
            #[serde(default)]
            timeline: Timeline,
        }

        #[derive(Debug, Default, Deserialize)]
        struct Timeline {
            #[serde(default)]
            events: Vec<TimelineEvent>,
        }

        #[derive(Debug, Deserialize)]
        struct TimelineEvent {
            #[serde(rename = "type")]
            event_type: String,
            event_id: String,
            sender: String,
            origin_server_ts: i64,
            #[serde(default)]
            content: TimelineContent,
        }

        #[derive(Debug, Default, Deserialize)]
        struct TimelineContent {
            #[serde(default)]
            body: String,
            #[serde(default, rename = "m.relates_to")]
            relates_to: Option<RelatesTo>,
        }

        #[derive(Debug, Deserialize)]
        struct RelatesTo {
            #[serde(default, rename = "m.in_reply_to")]
            in_reply_to: Option<ReplyTo>,
        }

        #[derive(Debug, Deserialize)]
        struct ReplyTo {
            event_id: String,
        }

        let since = self.sync_token.lock().await.clone();
        let mut url = self.client_api_url(&["sync"])?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("timeout", "1");
            if let Some(since) = since {
                query.append_pair("since", &since);
            }
        }

        let response = self
            .client
            .get(url)
            .bearer_auth(self.access_token()?)
            .send()
            .await
            .context("Matrix sync request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Matrix sync failed {}: {}", status, body);
        }

        let sync: SyncResponse = response.json().await.context("Invalid Matrix sync response")?;
        *self.sync_token.lock().await = Some(sync.next_batch);

        let mut messages = Vec::new();
        for (room_id, joined_room) in sync.rooms.join {
            for event in joined_room.timeline.events {
                if event.event_type != "m.room.message" || event.content.body.is_empty() {
                    continue;
                }

                messages.push(Message {
                    id: event.event_id,
                    sender: event.sender,
                    content: event.content.body,
                    timestamp: event.origin_server_ts / 1000,
                    channel: Some(room_id.clone()),
                    reply_to: event
                        .content
                        .relates_to
                        .and_then(|r| r.in_reply_to)
                        .map(|r| r.event_id),
                    media: None,
                    is_direct: false,
                    reactions: None,
                });
            }
        }

        Ok(messages)
    }

    async fn join_room_if_needed(&self, recipient: &str) -> Result<String> {
        if recipient.starts_with('!') {
            return Ok(recipient.to_string());
        }

        let response = self
            .client
            .post(self.client_api_url(&["join", recipient])?)
            .bearer_auth(self.access_token()?)
            .send()
            .await
            .context("Matrix join request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Matrix join failed {}: {}", status, body);
        }

        #[derive(Deserialize)]
        struct JoinResponse {
            room_id: String,
        }

        let join: JoinResponse = response.json().await.context("Invalid Matrix join response")?;
        Ok(join.room_id)
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
        #[derive(Deserialize)]
        struct LoginResponse {
            access_token: String,
            user_id: String,
        }

        self.validate_config()?;

        let response = self
            .client
            .post(self.client_api_url(&["login"])? )
            .json(&json!({
                "type": "m.login.password",
                "identifier": {
                    "type": "m.id.user",
                    "user": self.username,
                },
                "password": self.password,
                "initial_device_display_name": self.name,
            }))
            .send()
            .await
            .context("Matrix login request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Matrix login failed {}: {}", status, body);
        }

        let login: LoginResponse = response.json().await.context("Invalid Matrix login response")?;
        self.access_token = Some(login.access_token);
        self.user_id = Some(login.user_id);

        *self.sync_token.lock().await = None;
        let _ = self.sync_once().await?;

        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        let room_id = self.join_room_if_needed(recipient).await?;
        let txn_id = self.txn_counter.fetch_add(1, Ordering::Relaxed).to_string();

        let response = self
            .client
            .put(self.client_api_url(&[
                "rooms",
                &room_id,
                "send",
                "m.room.message",
                &txn_id,
            ])?)
            .bearer_auth(self.access_token()?)
            .json(&json!({
                "msgtype": "m.text",
                "body": content,
            }))
            .send()
            .await
            .context("Matrix send request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Matrix send failed {}: {}", status, body);
        }

        #[derive(Deserialize)]
        struct SendResponse {
            event_id: String,
        }

        let send: SendResponse = response.json().await.context("Invalid Matrix send response")?;
        Ok(send.event_id)
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        self.sync_once().await
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(token) = self.access_token.as_deref() {
            let response = self
                .client
                .post(self.client_api_url(&["logout"])? )
                .bearer_auth(token)
                .send()
                .await;

            if let Err(error) = response {
                tracing::warn!(messenger = %self.name, "Matrix logout failed: {error}");
            }
        }

        self.access_token = None;
        self.user_id = None;
        *self.sync_token.lock().await = None;
        self.connected = false;
        Ok(())
    }

    async fn set_typing(&self, channel: &str, typing: bool) -> Result<()> {
        let room_id = self.join_room_if_needed(channel).await?;
        let mut payload = json!({ "typing": typing });
        if typing {
            payload["timeout"] = json!(30_000);
        }

        let response = self
            .client
            .put(self.client_api_url(&["rooms", &room_id, "typing", self.user_id()?])?)
            .bearer_auth(self.access_token()?)
            .json(&payload)
            .send()
            .await
            .context("Matrix typing request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Matrix typing failed {}: {}", status, body);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_config_rejects_empty_homeserver() {
        let messenger = MatrixMessenger::new("matrix", "", "bot", "secret");
        assert!(messenger.validate_config().is_err());
    }

    #[test]
    fn validate_config_rejects_empty_username() {
        let messenger = MatrixMessenger::new("matrix", "https://matrix.example", "", "secret");
        assert!(messenger.validate_config().is_err());
    }

    #[test]
    fn validate_config_rejects_empty_password() {
        let messenger = MatrixMessenger::new("matrix", "https://matrix.example", "bot", "");
        assert!(messenger.validate_config().is_err());
    }

    #[test]
    fn validate_config_accepts_non_empty_values() {
        let messenger = MatrixMessenger::new(
            "matrix",
            "https://matrix.example",
            "bot",
            "secret",
        );
        assert!(messenger.validate_config().is_ok());
    }
}
