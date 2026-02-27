//! Thread events — emitted when thread state changes.

use super::{ThreadId, ThreadInfo, ThreadStatus};
use serde::{Deserialize, Serialize};

/// Events emitted by the ThreadManager.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ThreadEvent {
    /// A new thread was created
    Created {
        thread: ThreadInfo,
        parent_id: Option<ThreadId>,
    },
    
    /// Thread status changed
    StatusChanged {
        thread_id: ThreadId,
        old_status: ThreadStatus,
        new_status: ThreadStatus,
    },
    
    /// Thread description updated
    DescriptionChanged {
        thread_id: ThreadId,
        description: String,
    },
    
    /// Thread became foreground
    Foregrounded {
        thread_id: ThreadId,
        previous_foreground: Option<ThreadId>,
    },
    
    /// Thread completed (with optional result)
    Completed {
        thread_id: ThreadId,
        summary: Option<String>,
        result: Option<String>,
    },
    
    /// Thread failed
    Failed {
        thread_id: ThreadId,
        error: String,
    },
    
    /// Thread was removed/cleaned up
    Removed {
        thread_id: ThreadId,
    },
    
    /// Message added to thread
    MessageAdded {
        thread_id: ThreadId,
        message_count: usize,
    },
}

impl ThreadEvent {
    /// Get the thread ID this event relates to.
    pub fn thread_id(&self) -> ThreadId {
        match self {
            Self::Created { thread, .. } => thread.id,
            Self::StatusChanged { thread_id, .. } => *thread_id,
            Self::DescriptionChanged { thread_id, .. } => *thread_id,
            Self::Foregrounded { thread_id, .. } => *thread_id,
            Self::Completed { thread_id, .. } => *thread_id,
            Self::Failed { thread_id, .. } => *thread_id,
            Self::Removed { thread_id } => *thread_id,
            Self::MessageAdded { thread_id, .. } => *thread_id,
        }
    }
    
    /// Is this an event that should trigger a sidebar update?
    pub fn triggers_sidebar_update(&self) -> bool {
        // All events except MessageAdded trigger sidebar updates
        !matches!(self, Self::MessageAdded { .. })
    }
}
