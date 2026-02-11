use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Represents a message in the messenger system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub content: String,
    pub timestamp: i64,
}

/// Trait for messenger implementations (OpenClaw compatible)
#[async_trait]
pub trait Messenger: Send + Sync {
    /// Get the messenger name
    fn name(&self) -> &str;

    /// Initialize the messenger
    async fn initialize(&mut self) -> Result<()>;

    /// Send a message
    async fn send_message(&self, recipient: &str, content: &str) -> Result<()>;

    /// Receive messages (non-blocking)
    async fn receive_messages(&self) -> Result<Vec<Message>>;

    /// Check if the messenger is connected
    fn is_connected(&self) -> bool;

    /// Disconnect the messenger
    async fn disconnect(&mut self) -> Result<()>;
}

/// Manager for multiple messengers
pub struct MessengerManager {
    messengers: Vec<Box<dyn Messenger>>,
}

impl MessengerManager {
    pub fn new() -> Self {
        Self {
            messengers: Vec::new(),
        }
    }

    /// Add a messenger to the manager
    pub fn add_messenger(&mut self, messenger: Box<dyn Messenger>) {
        self.messengers.push(messenger);
    }

    /// Initialize all messengers
    pub async fn initialize_all(&mut self) -> Result<()> {
        for messenger in &mut self.messengers {
            messenger.initialize().await?;
        }
        Ok(())
    }

    /// Get all messengers
    pub fn get_messengers(&self) -> &[Box<dyn Messenger>] {
        &self.messengers
    }

    /// Get a messenger by name
    pub fn get_messenger(&self, name: &str) -> Option<&dyn Messenger> {
        self.messengers.iter().find(|m| m.name() == name).map(|b| &**b)
    }

    /// Disconnect all messengers
    pub async fn disconnect_all(&mut self) -> Result<()> {
        for messenger in &mut self.messengers {
            messenger.disconnect().await?;
        }
        Ok(())
    }
}

impl Default for MessengerManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Dummy messenger for testing and demonstration
pub struct DummyMessenger {
    name: String,
    connected: bool,
}

impl DummyMessenger {
    pub fn new(name: String) -> Self {
        Self {
            name,
            connected: false,
        }
    }
}

#[async_trait]
impl Messenger for DummyMessenger {
    fn name(&self) -> &str {
        &self.name
    }

    async fn initialize(&mut self) -> Result<()> {
        self.connected = true;
        Ok(())
    }

    async fn send_message(&self, _recipient: &str, _content: &str) -> Result<()> {
        // Dummy implementation
        Ok(())
    }

    async fn receive_messages(&self) -> Result<Vec<Message>> {
        // Dummy implementation
        Ok(Vec::new())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }
}
