//! iMessage messenger — macOS Messages.app integration.

use crate::{Message, Messenger};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use tokio::sync::Mutex;

#[cfg(target_os = "macos")]
use rusqlite::{params, Connection};

pub struct IMessageMessenger {
    name: String,
    chat_db_path: PathBuf,
    last_seen_rowid: Mutex<Option<i64>>,
    connected: bool,
}

impl IMessageMessenger {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            chat_db_path: default_chat_db_path(),
            last_seen_rowid: Mutex::new(None),
            connected: false,
        }
    }

    pub fn with_chat_db_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.chat_db_path = path.into();
        self
    }

    #[cfg(target_os = "macos")]
    fn max_rowid(path: &PathBuf) -> Result<Option<i64>> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open iMessage database at {}", path.display()))?;
        let rowid = conn.query_row("SELECT MAX(ROWID) FROM message", [], |row| row.get(0))?;
        Ok(rowid)
    }

    #[cfg(target_os = "macos")]
    fn fetch_messages(path: &PathBuf, since_rowid: i64, own_name: &str) -> Result<(Vec<Message>, Option<i64>)> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open iMessage database at {}", path.display()))?;

        let mut stmt = conn.prepare(
            "SELECT
                m.ROWID,
                COALESCE(m.guid, printf('imessage:%lld', m.ROWID)) AS guid,
                COALESCE(m.text, '') AS text,
                COALESCE(h.id, '') AS sender_handle,
                COALESCE(c.chat_identifier, h.id, '') AS channel_id,
                COALESCE(c.display_name, '') AS display_name,
                COALESCE(m.is_from_me, 0) AS is_from_me,
                COALESCE(m.thread_originator_guid, '') AS reply_to,
                CASE
                    WHEN m.date > 1000000000000 THEN (m.date / 1000000000) + 978307200
                    WHEN m.date > 0 THEN m.date + 978307200
                    ELSE strftime('%s','now')
                END AS unix_ts
             FROM message m
             LEFT JOIN handle h ON h.ROWID = m.handle_id
             LEFT JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
             LEFT JOIN chat c ON c.ROWID = cmj.chat_id
             WHERE m.ROWID > ?1 AND COALESCE(m.text, '') <> ''
             GROUP BY m.ROWID
             ORDER BY m.ROWID ASC",
        )?;

        let mut rows = stmt.query(params![since_rowid])?;
        let mut messages = Vec::new();
        let mut max_rowid = None;

        while let Some(row) = rows.next()? {
            let rowid: i64 = row.get(0)?;
            let guid: String = row.get(1)?;
            let text: String = row.get(2)?;
            let sender_handle: String = row.get(3)?;
            let channel_id: String = row.get(4)?;
            let display_name: String = row.get(5)?;
            let is_from_me: i64 = row.get(6)?;
            let reply_to: String = row.get(7)?;
            let unix_ts: i64 = row.get(8)?;

            max_rowid = Some(rowid);
            messages.push(Message {
                id: guid,
                sender: if is_from_me != 0 {
                    own_name.to_string()
                } else if sender_handle.is_empty() {
                    "unknown".to_string()
                } else {
                    sender_handle.clone()
                },
                content: text,
                timestamp: unix_ts,
                channel: if channel_id.is_empty() { None } else { Some(channel_id) },
                reply_to: if reply_to.is_empty() { None } else { Some(reply_to) },
                media: None,
                is_direct: display_name.is_empty(),
                reactions: None,
            });
        }

        Ok((messages, max_rowid))
    }

    #[cfg(target_os = "macos")]
    async fn send_via_applescript(&self, recipient: &str, content: &str) -> Result<String> {
        let script = format!(
            r#"tell application "Messages"
    set targetService to 1st service whose service type = iMessage
    set targetBuddy to buddy "{}" of targetService
    send "{}" to targetBuddy
end tell"#,
            escape_applescript_string(recipient),
            escape_applescript_string(content)
        );

        let output = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .await
            .context("Failed to launch osascript for iMessage send")?;

        if output.status.success() {
            Ok(format!("imessage:{}", chrono::Utc::now().timestamp_millis()))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("iMessage AppleScript failed: {}", stderr.trim());
        }
    }
}

#[async_trait]
impl Messenger for IMessageMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "imessage"
    }

    async fn initialize(&mut self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let path = self.chat_db_path.clone();
            if !path.exists() {
                anyhow::bail!(
                    "iMessage database not found at {}. Open Messages.app and allow Full Disk Access if needed.",
                    path.display()
                );
            }

            let max_rowid = tokio::task::spawn_blocking(move || Self::max_rowid(&path))
                .await
                .map_err(|e| anyhow!("Failed to join iMessage initialization task: {e}"))??;
            *self.last_seen_rowid.lock().await = max_rowid;
            self.connected = true;
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!("iMessage is only supported on macOS");
        }
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            self.send_via_applescript(recipient, content).await
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (recipient, content);
            anyhow::bail!("iMessage is only supported on macOS");
        }
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        #[cfg(target_os = "macos")]
        {
            if !self.connected {
                return Ok(Vec::new());
            }

            let since_rowid = self.last_seen_rowid.lock().await.unwrap_or(0);
            let path = self.chat_db_path.clone();
            let own_name = self.name.clone();
            let (messages, max_rowid) = tokio::task::spawn_blocking(move || {
                Self::fetch_messages(&path, since_rowid, &own_name)
            })
            .await
            .map_err(|e| anyhow!("Failed to join iMessage receive task: {e}"))??;
            if let Some(max_rowid) = max_rowid {
                *self.last_seen_rowid.lock().await = Some(max_rowid);
            }
            Ok(messages)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(Vec::new())
        }
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }
}

fn default_chat_db_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Library")
        .join("Messages")
        .join("chat.db")
}

fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_applescript_quotes_and_backslashes() {
        let escaped = escape_applescript_string(r#"a\b"c"#);
        assert_eq!(escaped, r#"a\\b\"c"#);
    }

    #[test]
    fn default_chat_db_path_points_to_messages_db() {
        let path = default_chat_db_path();
        assert!(path.ends_with(PathBuf::from("Library/Messages/chat.db")));
    }
}
