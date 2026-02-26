//! Thread manager — manages all agent threads.

use super::{AgentThread, MessageRole, ThreadEvent, ThreadId, ThreadInfo, ThreadKind, ThreadStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, warn};

/// Configuration for thread management.
#[derive(Debug, Clone)]
pub struct ThreadManagerConfig {
    /// How long to keep completed ephemeral threads before cleanup
    pub ephemeral_retention: Duration,
    
    /// Maximum messages before compaction is triggered
    pub compaction_threshold: usize,
    
    /// How many recent messages to keep after compaction
    pub messages_after_compaction: usize,
}

impl Default for ThreadManagerConfig {
    fn default() -> Self {
        Self {
            ephemeral_retention: Duration::from_secs(300), // 5 minutes
            compaction_threshold: 20,
            messages_after_compaction: 5,
        }
    }
}

/// Manages all agent threads with event emission.
pub struct ThreadManager {
    /// All threads by ID
    threads: HashMap<ThreadId, AgentThread>,
    
    /// Currently foregrounded thread
    foreground_id: Option<ThreadId>,
    
    /// Event broadcast channel
    events_tx: broadcast::Sender<ThreadEvent>,
    
    /// Configuration
    config: ThreadManagerConfig,
}

impl ThreadManager {
    /// Create a new thread manager.
    pub fn new() -> Self {
        Self::with_config(ThreadManagerConfig::default())
    }
    
    /// Create with custom config.
    pub fn with_config(config: ThreadManagerConfig) -> Self {
        let (events_tx, _) = broadcast::channel(256);
        Self {
            threads: HashMap::new(),
            foreground_id: None,
            events_tx,
            config,
        }
    }
    
    /// Subscribe to thread events.
    pub fn subscribe(&self) -> broadcast::Receiver<ThreadEvent> {
        self.events_tx.subscribe()
    }
    
    // ── Thread Creation ─────────────────────────────────────────────────────
    
    /// Create a new chat thread and make it foreground.
    pub fn create_chat(&mut self, label: impl Into<String>) -> ThreadId {
        let mut thread = AgentThread::new_chat(label);
        thread.is_foreground = true;
        let id = thread.id;
        
        // Background the old foreground
        if let Some(old_fg) = self.foreground_id {
            if let Some(t) = self.threads.get_mut(&old_fg) {
                t.is_foreground = false;
            }
        }
        
        self.foreground_id = Some(id);
        let info = thread.to_info();
        self.threads.insert(id, thread);
        
        self.emit(ThreadEvent::Created {
            thread: info,
            parent_id: None,
        });
        
        id
    }
    
    /// Create a sub-agent thread.
    pub fn create_subagent(
        &mut self,
        label: impl Into<String>,
        agent_id: impl Into<String>,
        task: impl Into<String>,
        parent_id: Option<ThreadId>,
    ) -> ThreadId {
        let thread = AgentThread::new_subagent(label, agent_id, task, parent_id);
        let id = thread.id;
        let info = thread.to_info();
        
        self.threads.insert(id, thread);
        
        self.emit(ThreadEvent::Created {
            thread: info,
            parent_id,
        });
        
        id
    }
    
    /// Create a background thread.
    pub fn create_background(
        &mut self,
        label: impl Into<String>,
        purpose: impl Into<String>,
        parent_id: Option<ThreadId>,
    ) -> ThreadId {
        let thread = AgentThread::new_background(label, purpose, parent_id);
        let id = thread.id;
        let info = thread.to_info();
        
        self.threads.insert(id, thread);
        
        self.emit(ThreadEvent::Created {
            thread: info,
            parent_id,
        });
        
        id
    }
    
    /// Create a task thread.
    pub fn create_task(
        &mut self,
        label: impl Into<String>,
        action: impl Into<String>,
        parent_id: Option<ThreadId>,
    ) -> ThreadId {
        let thread = AgentThread::new_task(label, action, parent_id);
        let id = thread.id;
        let info = thread.to_info();
        
        self.threads.insert(id, thread);
        
        self.emit(ThreadEvent::Created {
            thread: info,
            parent_id,
        });
        
        id
    }
    
    // ── Thread Access ───────────────────────────────────────────────────────
    
    /// Get a thread by ID.
    pub fn get(&self, id: ThreadId) -> Option<&AgentThread> {
        self.threads.get(&id)
    }
    
    /// Get a mutable thread by ID.
    pub fn get_mut(&mut self, id: ThreadId) -> Option<&mut AgentThread> {
        self.threads.get_mut(&id)
    }
    
    /// Get the foreground thread.
    pub fn foreground(&self) -> Option<&AgentThread> {
        self.foreground_id.and_then(|id| self.threads.get(&id))
    }
    
    /// Get the foreground thread mutably.
    pub fn foreground_mut(&mut self) -> Option<&mut AgentThread> {
        self.foreground_id.and_then(|id| self.threads.get_mut(&id))
    }
    
    /// Get foreground thread ID.
    pub fn foreground_id(&self) -> Option<ThreadId> {
        self.foreground_id
    }
    
    /// List all threads.
    pub fn list(&self) -> Vec<&AgentThread> {
        self.threads.values().collect()
    }
    
    /// List thread info for sidebar display.
    pub fn list_info(&self) -> Vec<ThreadInfo> {
        self.threads.values().map(|t| t.to_info()).collect()
    }
    
    // ── Thread Updates ──────────────────────────────────────────────────────
    
    /// Set description for a thread.
    pub fn set_description(&mut self, id: ThreadId, description: impl Into<String>) {
        let desc = description.into();
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.set_description(&desc);
            self.emit(ThreadEvent::DescriptionChanged {
                thread_id: id,
                description: desc,
            });
        }
    }
    
    /// Set description for the foreground thread.
    pub fn set_foreground_description(&mut self, description: impl Into<String>) {
        if let Some(id) = self.foreground_id {
            self.set_description(id, description);
        }
    }
    
    /// Update thread status.
    pub fn set_status(&mut self, id: ThreadId, status: ThreadStatus) {
        if let Some(thread) = self.threads.get_mut(&id) {
            let old_status = thread.status.clone();
            thread.set_status(status.clone());
            self.emit(ThreadEvent::StatusChanged {
                thread_id: id,
                old_status,
                new_status: status,
            });
        }
    }
    
    /// Switch foreground to a different thread.
    pub fn switch_foreground(&mut self, id: ThreadId) -> bool {
        if !self.threads.contains_key(&id) {
            return false;
        }
        
        let old_fg = self.foreground_id;
        
        // Background the old foreground
        if let Some(old_id) = old_fg {
            if let Some(t) = self.threads.get_mut(&old_id) {
                t.is_foreground = false;
            }
        }
        
        // Foreground the new one
        if let Some(t) = self.threads.get_mut(&id) {
            t.is_foreground = true;
        }
        
        self.foreground_id = Some(id);
        
        self.emit(ThreadEvent::Foregrounded {
            thread_id: id,
            previous_foreground: old_fg,
        });
        
        true
    }
    
    /// Rename a thread.
    pub fn rename(&mut self, id: ThreadId, new_label: impl Into<String>) -> bool {
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.label = new_label.into();
            thread.last_activity = SystemTime::now();
            true
        } else {
            false
        }
    }
    
    /// Find the best matching thread for a given message content.
    /// Returns the thread ID if a good match is found, None otherwise.
    pub fn find_best_match(&self, content: &str) -> Option<ThreadId> {
        let content_lower = content.to_lowercase();
        
        // Look for threads where label or description matches content keywords
        for thread in self.threads.values() {
            // Skip the current foreground
            if thread.is_foreground {
                continue;
            }
            
            // Check if thread label is mentioned in content
            if content_lower.contains(&thread.label.to_lowercase()) {
                return Some(thread.id);
            }
            
            // Check if description keywords match
            if let Some(desc) = &thread.description {
                let desc_words: Vec<&str> = desc.split_whitespace()
                    .filter(|w| w.len() > 3)
                    .collect();
                let matches = desc_words.iter()
                    .filter(|w| content_lower.contains(&w.to_lowercase()))
                    .count();
                if matches >= 2 {
                    return Some(thread.id);
                }
            }
        }
        
        None
    }
    
    /// Mark a thread as completed.
    pub fn complete(&mut self, id: ThreadId, summary: Option<String>, result: Option<String>) {
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.complete(summary.clone(), result.clone());
            self.emit(ThreadEvent::Completed {
                thread_id: id,
                summary,
                result,
            });
        }
    }
    
    /// Mark a thread as failed.
    pub fn fail(&mut self, id: ThreadId, error: impl Into<String>) {
        let err = error.into();
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.fail(&err);
            self.emit(ThreadEvent::Failed {
                thread_id: id,
                error: err,
            });
        }
    }
    
    /// Add a message to a thread.
    pub fn add_message(&mut self, id: ThreadId, role: MessageRole, content: impl Into<String>) {
        let message_count = if let Some(thread) = self.threads.get_mut(&id) {
            thread.add_message(role, content);
            thread.messages.len()
        } else {
            return;
        };
        
        self.emit(ThreadEvent::MessageAdded {
            thread_id: id,
            message_count,
        });
    }
    
    /// Add a message to the foreground thread.
    pub fn add_foreground_message(&mut self, role: MessageRole, content: impl Into<String>) {
        if let Some(id) = self.foreground_id {
            self.add_message(id, role, content);
        }
    }
    
    // ── Thread Removal ──────────────────────────────────────────────────────
    
    /// Remove a thread.
    pub fn remove(&mut self, id: ThreadId) -> Option<AgentThread> {
        let thread = self.threads.remove(&id);
        if thread.is_some() {
            if self.foreground_id == Some(id) {
                self.foreground_id = None;
            }
            self.emit(ThreadEvent::Removed { thread_id: id });
        }
        thread
    }
    
    /// Clean up old ephemeral threads.
    pub fn cleanup_ephemeral(&mut self) {
        let now = SystemTime::now();
        let retention = self.config.ephemeral_retention;
        
        let to_remove: Vec<ThreadId> = self.threads
            .iter()
            .filter(|(_, t)| {
                t.kind.is_ephemeral()
                    && t.status.is_terminal()
                    && now.duration_since(t.last_activity)
                        .map(|d| d > retention)
                        .unwrap_or(false)
            })
            .map(|(id, _)| *id)
            .collect();
        
        for id in to_remove {
            self.remove(id);
        }
    }
    
    // ── Context Building ────────────────────────────────────────────────────
    
    /// Build global context from all threads that share context.
    pub fn build_global_context(&self) -> String {
        let mut context = String::new();
        
        for thread in self.threads.values() {
            if !thread.share_context || thread.is_foreground {
                continue;
            }
            
            // Include summary or recent info for backgrounded threads
            if let Some(summary) = &thread.compact_summary {
                context.push_str(&format!(
                    "## {} ({})\n{}\n\n",
                    thread.label,
                    thread.kind.display_name(),
                    summary
                ));
            } else if !thread.messages.is_empty() {
                let recent: Vec<_> = thread.messages.iter().rev().take(2).collect();
                context.push_str(&format!(
                    "## {} ({}) - {} messages\n",
                    thread.label,
                    thread.kind.display_name(),
                    thread.messages.len()
                ));
                for msg in recent.into_iter().rev() {
                    context.push_str(&format!("{:?}: {}\n", msg.role, &msg.content[..msg.content.len().min(100)]));
                }
                context.push('\n');
            }
        }
        
        context
    }
    
    // ── Persistence ─────────────────────────────────────────────────────────
    
    /// Save threads to a file.
    pub fn save_to_file(&self, path: &Path) -> std::io::Result<()> {
        let state = PersistentState {
            threads: self.threads.values().cloned().collect(),
            foreground_id: self.foreground_id,
        };
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }
    
    /// Load threads from a file.
    pub fn load_from_file(path: &Path) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let state: PersistentState = serde_json::from_str(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        
        let (events_tx, _) = broadcast::channel(256);
        let mut mgr = Self {
            threads: HashMap::new(),
            foreground_id: state.foreground_id,
            events_tx,
            config: ThreadManagerConfig::default(),
        };
        
        for thread in state.threads {
            mgr.threads.insert(thread.id, thread);
        }
        
        Ok(mgr)
    }
    
    /// Load from file or create with default chat thread.
    pub fn load_or_default(path: &Path) -> Self {
        match Self::load_from_file(path) {
            Ok(mgr) => {
                debug!("Loaded {} threads from {:?}", mgr.threads.len(), path);
                mgr
            }
            Err(e) => {
                debug!("Creating new thread manager (load failed: {})", e);
                let mut mgr = Self::new();
                mgr.create_chat("Main");
                mgr
            }
        }
    }
    
    // ── Internal ────────────────────────────────────────────────────────────
    
    fn emit(&self, event: ThreadEvent) {
        // Ignore send errors (no subscribers)
        let _ = self.events_tx.send(event);
    }
}

impl Default for ThreadManager {
    fn default() -> Self {
        Self::new()
    }
}

/// State for persistence.
#[derive(Debug, Serialize, Deserialize)]
struct PersistentState {
    threads: Vec<AgentThread>,
    foreground_id: Option<ThreadId>,
}

/// Shared thread manager type.
pub type SharedThreadManager = Arc<RwLock<ThreadManager>>;

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_create_chat_thread() {
        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Test Chat");
        
        assert!(mgr.get(id).is_some());
        assert_eq!(mgr.foreground_id(), Some(id));
        assert!(mgr.get(id).unwrap().is_foreground);
    }
    
    #[test]
    fn test_create_subagent() {
        let mut mgr = ThreadManager::new();
        let parent = mgr.create_chat("Main");
        let child = mgr.create_subagent("Worker", "gpt-4", "Do the thing", Some(parent));
        
        let thread = mgr.get(child).unwrap();
        assert!(matches!(thread.kind, ThreadKind::SubAgent { .. }));
        assert_eq!(thread.parent_id, Some(parent));
    }
    
    #[test]
    fn test_switch_foreground() {
        let mut mgr = ThreadManager::new();
        let id1 = mgr.create_chat("Chat 1");
        let id2 = mgr.create_chat("Chat 2");
        
        assert_eq!(mgr.foreground_id(), Some(id2));
        assert!(!mgr.get(id1).unwrap().is_foreground);
        
        mgr.switch_foreground(id1);
        assert_eq!(mgr.foreground_id(), Some(id1));
        assert!(mgr.get(id1).unwrap().is_foreground);
        assert!(!mgr.get(id2).unwrap().is_foreground);
    }
    
    #[test]
    fn test_set_description() {
        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Test");
        
        mgr.set_description(id, "Working on taxes");
        assert_eq!(mgr.get(id).unwrap().description.as_deref(), Some("Working on taxes"));
    }
    
    #[test]
    fn test_complete_and_fail() {
        let mut mgr = ThreadManager::new();
        
        let task1 = mgr.create_task("Task 1", "Do thing", None);
        mgr.complete(task1, Some("Done!".into()), Some("result data".into()));
        assert!(mgr.get(task1).unwrap().status.is_terminal());
        assert_eq!(mgr.get(task1).unwrap().result.as_deref(), Some("result data"));
        
        let task2 = mgr.create_task("Task 2", "Do other thing", None);
        mgr.fail(task2, "Something went wrong");
        assert!(matches!(mgr.get(task2).unwrap().status, ThreadStatus::Failed { .. }));
    }
    
    #[test]
    fn test_list_info() {
        let mut mgr = ThreadManager::new();
        mgr.create_chat("Chat");
        mgr.create_subagent("Worker", "gpt-4", "task", None);
        
        let info = mgr.list_info();
        assert_eq!(info.len(), 2);
    }
}
