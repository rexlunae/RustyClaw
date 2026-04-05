//! Console messenger — stdin/stdout implementation for testing.

use crate::{Message, Messenger};
use anyhow::Result;
use async_trait::async_trait;

pub struct ConsoleMessenger {
    name: String,
    connected: bool,
    messages: Vec<Message>,
}

impl ConsoleMessenger {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            connected: false,
            messages: Vec::new(),
        }
    }

    /// Queue a message to be returned by `receive_messages`.
    pub fn enqueue(&mut self, message: Message) {
        self.messages.push(message);
    }
}

#[async_trait]
impl Messenger for ConsoleMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    fn messenger_type(&self) -> &str {
        "console"
    }

    async fn initialize(&mut self) -> Result<()> {
        self.connected = true;
        println!("[{}] Console messenger initialized.", self.name);
        Ok(())
    }

    async fn send_message(&self, recipient: &str, content: &str) -> Result<String> {
        println!("[{}] → {}: {}", self.name, recipient, content);
        Ok(format!("console:{}", chrono::Utc::now().timestamp_millis()))
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        // Clone is intentional: ConsoleMessenger is a test double and callers
        // may call receive_messages multiple times against the same queued data.
        Ok(self.messages.clone())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        println!("[{}] Console messenger disconnected.", self.name);
        Ok(())
    }
}
