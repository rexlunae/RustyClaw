//! Task manager — orchestrates task lifecycle and events.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, broadcast};
use tracing::{debug, info, warn, instrument};

use super::model::{Task, TaskId, TaskKind, TaskStatus, TaskProgress};

/// Handle to a running task for control and monitoring.
pub struct TaskHandle {
    /// Task ID
    pub id: TaskId,
    
    /// Channel to send control commands
    control_tx: mpsc::Sender<TaskControl>,
    
    /// Channel to receive output (not Clone, so we store the sender to resubscribe)
    output_tx: broadcast::Sender<TaskOutput>,
}

impl Clone for TaskHandle {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            control_tx: self.control_tx.clone(),
            output_tx: self.output_tx.clone(),
        }
    }
}

impl TaskHandle {
    /// Subscribe to task output.
    pub fn subscribe(&self) -> broadcast::Receiver<TaskOutput> {
        self.output_tx.subscribe()
    }

    /// Send a control command to the task.
    pub async fn send_control(&self, cmd: TaskControl) -> Result<(), mpsc::error::SendError<TaskControl>> {
        self.control_tx.send(cmd).await
    }

    /// Request the task to pause.
    pub async fn pause(&self) -> Result<(), mpsc::error::SendError<TaskControl>> {
        self.send_control(TaskControl::Pause).await
    }

    /// Request the task to resume.
    pub async fn resume(&self) -> Result<(), mpsc::error::SendError<TaskControl>> {
        self.send_control(TaskControl::Resume).await
    }

    /// Request the task to cancel.
    pub async fn cancel(&self) -> Result<(), mpsc::error::SendError<TaskControl>> {
        self.send_control(TaskControl::Cancel).await
    }

    /// Move task to foreground.
    pub async fn foreground(&self) -> Result<(), mpsc::error::SendError<TaskControl>> {
        self.send_control(TaskControl::Foreground).await
    }

    /// Move task to background.
    pub async fn background(&self) -> Result<(), mpsc::error::SendError<TaskControl>> {
        self.send_control(TaskControl::Background).await
    }

    /// Send input to the task.
    pub async fn send_input(&self, input: String) -> Result<(), mpsc::error::SendError<TaskControl>> {
        self.send_control(TaskControl::Input(input)).await
    }
}

/// Control commands that can be sent to a task.
#[derive(Debug, Clone)]
pub enum TaskControl {
    /// Pause the task
    Pause,
    /// Resume a paused task
    Resume,
    /// Cancel the task
    Cancel,
    /// Move to foreground (stream output)
    Foreground,
    /// Move to background (buffer output)
    Background,
    /// Send input to the task
    Input(String),
    /// Update progress
    Progress(TaskProgress),
}

/// Output events from a task.
#[derive(Debug, Clone)]
pub enum TaskOutput {
    /// Standard output line
    Stdout(String),
    /// Standard error line
    Stderr(String),
    /// Progress update
    Progress(TaskProgress),
    /// Status change
    StatusChange(TaskStatus),
    /// Task completed
    Completed { summary: Option<String>, output: Option<String> },
    /// Task failed
    Failed { error: String, retryable: bool },
}

/// Events emitted by the task manager.
#[derive(Debug, Clone)]
pub enum TaskEvent {
    /// A new task was created
    Created(Task),
    /// Task status changed
    StatusChanged { id: TaskId, old: TaskStatus, new: TaskStatus },
    /// Task produced output
    Output { id: TaskId, output: TaskOutput },
    /// Task was foregrounded
    Foregrounded(TaskId),
    /// Task was backgrounded
    Backgrounded(TaskId),
    /// Task completed
    Completed(Task),
    /// Task failed
    Failed(Task),
    /// Task was cancelled
    Cancelled(TaskId),
}

/// Manages all tasks across sessions.
pub struct TaskManager {
    /// All tasks by ID
    tasks: Arc<RwLock<HashMap<TaskId, Task>>>,
    
    /// Control channels by task ID
    controls: Arc<RwLock<HashMap<TaskId, mpsc::Sender<TaskControl>>>>,
    
    /// Output broadcast channels by task ID
    outputs: Arc<RwLock<HashMap<TaskId, broadcast::Sender<TaskOutput>>>>,
    
    /// Event broadcast channel
    events_tx: broadcast::Sender<TaskEvent>,
    
    /// Currently foregrounded task per session
    foreground_tasks: Arc<RwLock<HashMap<String, TaskId>>>,
}

impl TaskManager {
    /// Create a new task manager.
    pub fn new() -> Self {
        let (events_tx, _) = broadcast::channel(256);
        
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            controls: Arc::new(RwLock::new(HashMap::new())),
            outputs: Arc::new(RwLock::new(HashMap::new())),
            events_tx,
            foreground_tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Subscribe to task events.
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.events_tx.subscribe()
    }

    /// Create a new task and return its handle.
    #[instrument(skip(self), fields(kind = ?kind))]
    pub async fn create(&self, kind: TaskKind, session_key: Option<String>) -> TaskHandle {
        let task = Task::new(kind).with_session(session_key.clone().unwrap_or_default());
        let id = task.id;
        
        // Create control channel
        let (control_tx, _control_rx) = mpsc::channel(32);
        
        // Create output broadcast channel
        let (output_tx, output_rx) = broadcast::channel(256);
        
        // Store task
        self.tasks.write().await.insert(id, task.clone());
        self.controls.write().await.insert(id, control_tx.clone());
        self.outputs.write().await.insert(id, output_tx);
        
        // Emit event
        let _ = self.events_tx.send(TaskEvent::Created(task));
        
        debug!(task_id = %id, "Task created");
        
        TaskHandle {
            id,
            control_tx,
            output_tx,
        }
    }

    /// Get a task by ID.
    pub async fn get(&self, id: TaskId) -> Option<Task> {
        self.tasks.read().await.get(&id).cloned()
    }

    /// Get all tasks.
    pub async fn all(&self) -> Vec<Task> {
        self.tasks.read().await.values().cloned().collect()
    }

    /// Get tasks for a specific session.
    pub async fn for_session(&self, session_key: &str) -> Vec<Task> {
        self.tasks.read().await
            .values()
            .filter(|t| t.session_key.as_deref() == Some(session_key))
            .cloned()
            .collect()
    }

    /// Get active (non-terminal) tasks.
    pub async fn active(&self) -> Vec<Task> {
        self.tasks.read().await
            .values()
            .filter(|t| !t.status.is_terminal())
            .cloned()
            .collect()
    }

    /// Get the foreground task for a session.
    pub async fn foreground_task(&self, session_key: &str) -> Option<Task> {
        let fg_id = self.foreground_tasks.read().await.get(session_key).copied()?;
        self.get(fg_id).await
    }

    /// Set a task as the foreground task for its session.
    #[instrument(skip(self))]
    pub async fn set_foreground(&self, id: TaskId) -> Result<(), String> {
        let mut tasks = self.tasks.write().await;
        
        // Get session key first
        let session = {
            let task = tasks.get(&id)
                .ok_or_else(|| format!("Task {} not found", id))?;
            task.session_key.clone()
                .ok_or_else(|| "Task has no session".to_string())?
        };
        
        // Background the current foreground task if any
        if let Some(old_fg_id) = self.foreground_tasks.read().await.get(&session).copied() {
            if old_fg_id != id {
                if let Some(old_task) = tasks.get_mut(&old_fg_id) {
                    old_task.background();
                    let _ = self.events_tx.send(TaskEvent::Backgrounded(old_fg_id));
                }
            }
        }
        
        // Foreground this task
        if let Some(task) = tasks.get_mut(&id) {
            task.foreground();
        }
        self.foreground_tasks.write().await.insert(session, id);
        let _ = self.events_tx.send(TaskEvent::Foregrounded(id));
        
        info!(task_id = %id, "Task foregrounded");
        Ok(())
    }

    /// Background a task.
    #[instrument(skip(self))]
    pub async fn set_background(&self, id: TaskId) -> Result<(), String> {
        let mut tasks = self.tasks.write().await;
        let task = tasks.get_mut(&id)
            .ok_or_else(|| format!("Task {} not found", id))?;
        
        task.background();
        
        // Remove from foreground if it was there
        if let Some(ref session) = task.session_key {
            let mut fg = self.foreground_tasks.write().await;
            if fg.get(session) == Some(&id) {
                fg.remove(session);
            }
        }
        
        let _ = self.events_tx.send(TaskEvent::Backgrounded(id));
        info!(task_id = %id, "Task backgrounded");
        Ok(())
    }

    /// Update a task's status.
    #[instrument(skip(self))]
    pub async fn update_status(&self, id: TaskId, new_status: TaskStatus) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(&id) {
            let old_status = task.status.clone();
            task.status = new_status.clone();
            
            if new_status.is_terminal() {
                task.finished_at = Some(std::time::SystemTime::now());
            }
            
            let _ = self.events_tx.send(TaskEvent::StatusChanged {
                id,
                old: old_status,
                new: new_status,
            });
        }
    }

    /// Mark a task as started.
    pub async fn start(&self, id: TaskId) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(&id) {
            task.start();
            
            // Auto-foreground if no foreground task for this session
            if let Some(ref session) = task.session_key {
                let mut fg = self.foreground_tasks.write().await;
                if !fg.contains_key(session) {
                    fg.insert(session.clone(), id);
                    let _ = self.events_tx.send(TaskEvent::Foregrounded(id));
                }
            }
        }
    }

    /// Mark a task as completed.
    pub async fn complete(&self, id: TaskId, summary: Option<String>) {
        let task = {
            let mut tasks = self.tasks.write().await;
            if let Some(task) = tasks.get_mut(&id) {
                task.complete(summary);
                
                // Remove from foreground
                if let Some(ref session) = task.session_key {
                    self.foreground_tasks.write().await.remove(session);
                }
                
                Some(task.clone())
            } else {
                None
            }
        };
        
        if let Some(t) = task {
            let _ = self.events_tx.send(TaskEvent::Completed(t));
            info!(task_id = %id, "Task completed");
        }
    }

    /// Mark a task as failed.
    pub async fn fail(&self, id: TaskId, error: String, retryable: bool) {
        let task = {
            let mut tasks = self.tasks.write().await;
            if let Some(task) = tasks.get_mut(&id) {
                task.fail(&error, retryable);
                
                // Remove from foreground
                if let Some(ref session) = task.session_key {
                    self.foreground_tasks.write().await.remove(session);
                }
                
                Some(task.clone())
            } else {
                None
            }
        };
        
        if let Some(t) = task {
            let _ = self.events_tx.send(TaskEvent::Failed(t));
            warn!(task_id = %id, error = %error, "Task failed");
        }
    }

    /// Cancel a task.
    pub async fn cancel(&self, id: TaskId) -> Result<(), String> {
        // Send cancel command if task is running
        if let Some(control_tx) = self.controls.read().await.get(&id) {
            let _ = control_tx.send(TaskControl::Cancel).await;
        }
        
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(&id) {
            task.cancel();
            
            // Remove from foreground
            if let Some(ref session) = task.session_key {
                self.foreground_tasks.write().await.remove(session);
            }
            
            let _ = self.events_tx.send(TaskEvent::Cancelled(id));
            info!(task_id = %id, "Task cancelled");
            Ok(())
        } else {
            Err(format!("Task {} not found", id))
        }
    }

    /// Send output for a task.
    pub async fn send_output(&self, id: TaskId, output: TaskOutput) {
        if let Some(output_tx) = self.outputs.read().await.get(&id) {
            let _ = output_tx.send(output.clone());
        }
        
        let _ = self.events_tx.send(TaskEvent::Output { id, output });
    }

    /// Cleanup completed/cancelled tasks older than the given duration.
    pub async fn cleanup_old(&self, max_age: std::time::Duration) {
        let now = std::time::SystemTime::now();
        let mut tasks = self.tasks.write().await;
        
        let to_remove: Vec<TaskId> = tasks.iter()
            .filter(|(_, t)| {
                if !t.status.is_terminal() {
                    return false;
                }
                if let Some(finished) = t.finished_at {
                    now.duration_since(finished).unwrap_or_default() > max_age
                } else {
                    false
                }
            })
            .map(|(id, _)| *id)
            .collect();
        
        for id in &to_remove {
            tasks.remove(id);
            self.controls.write().await.remove(id);
            self.outputs.write().await.remove(id);
        }
        
        if !to_remove.is_empty() {
            debug!(count = to_remove.len(), "Cleaned up old tasks");
        }
    }

    /// Get task statistics.
    pub async fn stats(&self) -> TaskStats {
        let tasks = self.tasks.read().await;
        
        let mut stats = TaskStats::default();
        for task in tasks.values() {
            stats.total += 1;
            match &task.status {
                TaskStatus::Pending => stats.pending += 1,
                TaskStatus::Running { .. } => stats.running += 1,
                TaskStatus::Background { .. } => stats.background += 1,
                TaskStatus::Paused { .. } => stats.paused += 1,
                TaskStatus::Completed { .. } => stats.completed += 1,
                TaskStatus::Failed { .. } => stats.failed += 1,
                TaskStatus::Cancelled => stats.cancelled += 1,
                TaskStatus::WaitingForInput { .. } => stats.waiting_input += 1,
            }
        }
        
        stats
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Task statistics.
#[derive(Debug, Clone, Default)]
pub struct TaskStats {
    pub total: usize,
    pub pending: usize,
    pub running: usize,
    pub background: usize,
    pub paused: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub waiting_input: usize,
}

impl TaskStats {
    /// Get count of active (non-terminal) tasks.
    pub fn active(&self) -> usize {
        self.pending + self.running + self.background + self.paused + self.waiting_input
    }
}
