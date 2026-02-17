//! Routines engine for scheduled and event-driven automation.
//!
//! Provides automated task execution through:
//! 1. **Cron triggers**: Time-based scheduling with cron expressions
//! 2. **Event triggers**: Pattern-based matching on agent responses
//! 3. **Webhook triggers**: External HTTP triggers with HMAC validation
//! 4. **Manual triggers**: On-demand execution via CLI
//!
//! ## Database Schema
//!
//! ```sql
//! CREATE TABLE routines (
//!     id INTEGER PRIMARY KEY,
//!     name TEXT UNIQUE NOT NULL,
//!     description TEXT,
//!     prompt TEXT NOT NULL,           -- The AI prompt to execute
//!     enabled BOOLEAN DEFAULT 1,
//!     trigger_type TEXT NOT NULL,     -- 'cron', 'event', 'webhook', 'manual'
//!     trigger_config TEXT NOT NULL,   -- JSON config for the trigger
//!     max_failures INTEGER DEFAULT 3,
//!     cooldown_secs INTEGER DEFAULT 300,
//!     failure_count INTEGER DEFAULT 0,
//!     last_run INTEGER,
//!     last_success INTEGER,
//!     created_at INTEGER NOT NULL,
//!     updated_at INTEGER NOT NULL
//! );
//!
//! CREATE TABLE routine_executions (
//!     id INTEGER PRIMARY KEY,
//!     routine_id INTEGER NOT NULL,
//!     started_at INTEGER NOT NULL,
//!     completed_at INTEGER,
//!     status TEXT NOT NULL,           -- 'running', 'success', 'failed'
//!     trigger_source TEXT,            -- What triggered this execution
//!     error_message TEXT,
//!     output TEXT,
//!     FOREIGN KEY (routine_id) REFERENCES routines(id) ON DELETE CASCADE
//! );
//! ```
//!
//! ## Configuration Example
//!
//! ```toml
//! [routines]
//! enabled = true
//! db_path = "routines/routines.db"  # relative to workspace
//! check_interval_secs = 60           # How often to check for pending routines
//! webhook_secret = "your-secret-key" # HMAC secret for webhook validation
//! ```
//!
//! ## Usage Examples
//!
//! ### Cron Routine (Periodic Backups)
//! ```bash
//! rustyclaw routine create backup-daily \
//!   --cron "0 2 * * *" \
//!   --prompt "Create a backup of all workspace files to ~/backups"
//! ```
//!
//! ### Event Routine (Monitor GitHub)
//! ```bash
//! rustyclaw routine create github-alerts \
//!   --event "new.*issue|pull.*request" \
//!   --prompt "Check GitHub notifications and summarize any new activity"
//! ```
//!
//! ### Webhook Routine (External Integration)
//! ```bash
//! rustyclaw routine create deploy-hook \
//!   --webhook \
//!   --prompt "Run deployment checklist and notify team"
//! ```

use anyhow::{Context as AnyhowContext, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Routines engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutinesConfig {
    /// Whether routines engine is enabled
    #[serde(default)]
    pub enabled: bool,

    /// Database path (relative to workspace)
    #[serde(default = "RoutinesConfig::default_db_path")]
    pub db_path: String,

    /// How often to check for pending routines (seconds)
    #[serde(default = "RoutinesConfig::default_check_interval")]
    pub check_interval_secs: u64,

    /// HMAC secret for webhook validation
    #[serde(default)]
    pub webhook_secret: Option<String>,
}

impl RoutinesConfig {
    fn default_db_path() -> String {
        "routines/routines.db".to_string()
    }

    fn default_check_interval() -> u64 {
        60 // Check every minute
    }
}

impl Default for RoutinesConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            db_path: Self::default_db_path(),
            check_interval_secs: Self::default_check_interval(),
            webhook_secret: None,
        }
    }
}

/// Trigger type for a routine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TriggerType {
    /// Time-based cron trigger
    Cron,
    /// Event pattern trigger (regex)
    Event,
    /// Webhook HTTP trigger
    Webhook,
    /// Manual execution only
    Manual,
}

impl TriggerType {
    fn as_str(&self) -> &str {
        match self {
            TriggerType::Cron => "cron",
            TriggerType::Event => "event",
            TriggerType::Webhook => "webhook",
            TriggerType::Manual => "manual",
        }
    }

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "cron" => Ok(TriggerType::Cron),
            "event" => Ok(TriggerType::Event),
            "webhook" => Ok(TriggerType::Webhook),
            "manual" => Ok(TriggerType::Manual),
            _ => anyhow::bail!("Invalid trigger type: {}", s),
        }
    }
}

/// Trigger configuration (varies by type).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TriggerConfig {
    /// Cron expression (e.g., "0 9 * * MON-FRI")
    Cron { expression: String },
    /// Regex pattern to match in agent responses
    Event { pattern: String },
    /// Webhook endpoint configuration
    Webhook {
        /// Optional webhook path suffix (e.g., "/hook/my-routine")
        path: Option<String>,
    },
    /// Manual execution only (no automatic triggers)
    Manual,
}

/// Execution status for a routine run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionStatus {
    Running,
    Success,
    Failed,
}

impl ExecutionStatus {
    fn as_str(&self) -> &str {
        match self {
            ExecutionStatus::Running => "running",
            ExecutionStatus::Success => "success",
            ExecutionStatus::Failed => "failed",
        }
    }

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "running" => Ok(ExecutionStatus::Running),
            "success" => Ok(ExecutionStatus::Success),
            "failed" => Ok(ExecutionStatus::Failed),
            _ => anyhow::bail!("Invalid execution status: {}", s),
        }
    }
}

/// A routine definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routine {
    pub id: Option<i64>,
    pub name: String,
    pub description: Option<String>,
    pub prompt: String,
    pub enabled: bool,
    pub trigger_type: TriggerType,
    pub trigger_config: TriggerConfig,
    pub max_failures: i64,
    pub cooldown_secs: i64,
    pub failure_count: i64,
    pub last_run: Option<i64>,
    pub last_success: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Routine {
    /// Create a new routine.
    pub fn new(name: String, prompt: String, trigger_config: TriggerConfig) -> Self {
        let now = chrono::Utc::now().timestamp();
        let trigger_type = match &trigger_config {
            TriggerConfig::Cron { .. } => TriggerType::Cron,
            TriggerConfig::Event { .. } => TriggerType::Event,
            TriggerConfig::Webhook { .. } => TriggerType::Webhook,
            TriggerConfig::Manual => TriggerType::Manual,
        };

        Self {
            id: None,
            name,
            description: None,
            prompt,
            enabled: true,
            trigger_type,
            trigger_config,
            max_failures: 3,
            cooldown_secs: 300, // 5 minutes
            failure_count: 0,
            last_run: None,
            last_success: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set the description.
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    /// Set max failures before disabling.
    pub fn with_max_failures(mut self, max_failures: i64) -> Self {
        self.max_failures = max_failures;
        self
    }

    /// Set cooldown period in seconds.
    pub fn with_cooldown(mut self, cooldown_secs: i64) -> Self {
        self.cooldown_secs = cooldown_secs;
        self
    }

    /// Check if the routine is in cooldown (recently ran and failed).
    pub fn is_in_cooldown(&self) -> bool {
        if let Some(last_run) = self.last_run {
            let now = chrono::Utc::now().timestamp();
            let elapsed = now - last_run;
            elapsed < self.cooldown_secs
        } else {
            false
        }
    }

    /// Check if the routine should be disabled due to too many failures.
    pub fn should_disable(&self) -> bool {
        self.failure_count >= self.max_failures
    }
}

/// A routine execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineExecution {
    pub id: Option<i64>,
    pub routine_id: i64,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub status: ExecutionStatus,
    pub trigger_source: Option<String>,
    pub error_message: Option<String>,
    pub output: Option<String>,
}

impl RoutineExecution {
    /// Create a new execution record.
    pub fn new(routine_id: i64, trigger_source: Option<String>) -> Self {
        Self {
            id: None,
            routine_id,
            started_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            status: ExecutionStatus::Running,
            trigger_source,
            error_message: None,
            output: None,
        }
    }

    /// Mark the execution as successful.
    pub fn mark_success(mut self, output: Option<String>) -> Self {
        self.completed_at = Some(chrono::Utc::now().timestamp());
        self.status = ExecutionStatus::Success;
        self.output = output;
        self
    }

    /// Mark the execution as failed.
    pub fn mark_failed(mut self, error: String) -> Self {
        self.completed_at = Some(chrono::Utc::now().timestamp());
        self.status = ExecutionStatus::Failed;
        self.error_message = Some(error);
        self
    }

    /// Get the duration in seconds (if completed).
    pub fn duration_secs(&self) -> Option<i64> {
        self.completed_at.map(|end| end - self.started_at)
    }
}

/// Routines storage with SQLite backend.
pub struct RoutinesStore {
    db_path: PathBuf,
    conn: Arc<RwLock<Connection>>,
    config: RoutinesConfig,
}

impl RoutinesStore {
    /// Open or create a routines database.
    pub fn open(workspace: &Path, config: RoutinesConfig) -> Result<Self> {
        let db_path = workspace.join(&config.db_path);

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create routines directory: {}", parent.display()))?;
        }

        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

        // Initialize schema
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS routines (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT UNIQUE NOT NULL,
                description TEXT,
                prompt TEXT NOT NULL,
                enabled BOOLEAN DEFAULT 1,
                trigger_type TEXT NOT NULL,
                trigger_config TEXT NOT NULL,
                max_failures INTEGER DEFAULT 3,
                cooldown_secs INTEGER DEFAULT 300,
                failure_count INTEGER DEFAULT 0,
                last_run INTEGER,
                last_success INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS routine_executions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                routine_id INTEGER NOT NULL,
                started_at INTEGER NOT NULL,
                completed_at INTEGER,
                status TEXT NOT NULL,
                trigger_source TEXT,
                error_message TEXT,
                output TEXT,
                FOREIGN KEY (routine_id) REFERENCES routines(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_routines_enabled ON routines(enabled);
            CREATE INDEX IF NOT EXISTS idx_routines_trigger_type ON routines(trigger_type);
            CREATE INDEX IF NOT EXISTS idx_executions_routine ON routine_executions(routine_id);
            CREATE INDEX IF NOT EXISTS idx_executions_started ON routine_executions(started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_executions_status ON routine_executions(status);
            "#,
        )?;

        Ok(Self {
            db_path,
            conn: Arc::new(RwLock::new(conn)),
            config,
        })
    }

    /// Create a new routine.
    pub async fn create_routine(&self, mut routine: Routine) -> Result<i64> {
        let now = chrono::Utc::now().timestamp();
        routine.created_at = now;
        routine.updated_at = now;

        let trigger_config_json = serde_json::to_string(&routine.trigger_config)?;
        let conn = self.conn.write().await;

        let id = conn.execute(
            r#"
            INSERT INTO routines (
                name, description, prompt, enabled, trigger_type, trigger_config,
                max_failures, cooldown_secs, failure_count, last_run, last_success,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            params![
                routine.name,
                routine.description,
                routine.prompt,
                routine.enabled,
                routine.trigger_type.as_str(),
                trigger_config_json,
                routine.max_failures,
                routine.cooldown_secs,
                routine.failure_count,
                routine.last_run,
                routine.last_success,
                routine.created_at,
                routine.updated_at,
            ],
        )?;

        Ok(conn.last_insert_rowid())
    }

    /// Get a routine by ID.
    pub async fn get_routine(&self, id: i64) -> Result<Option<Routine>> {
        let conn = self.conn.read().await;
        let mut stmt = conn.prepare(
            "SELECT id, name, description, prompt, enabled, trigger_type, trigger_config, \
             max_failures, cooldown_secs, failure_count, last_run, last_success, \
             created_at, updated_at FROM routines WHERE id = ?1",
        )?;

        let routine = stmt
            .query_row(params![id], |row| {
                let trigger_config_json: String = row.get(6)?;
                let trigger_config: TriggerConfig = serde_json::from_str(&trigger_config_json)
                    .map_err(|_| rusqlite::Error::InvalidColumnType(6, "trigger_config".to_string(), rusqlite::types::Type::Text))?;

                Ok(Routine {
                    id: Some(row.get(0)?),
                    name: row.get(1)?,
                    description: row.get(2)?,
                    prompt: row.get(3)?,
                    enabled: row.get(4)?,
                    trigger_type: {
                        let type_str: String = row.get(5)?;
                        TriggerType::from_str(&type_str)
                            .map_err(|e| rusqlite::Error::InvalidColumnType(5, "trigger_type".to_string(), rusqlite::types::Type::Text))?
                    },
                    trigger_config,
                    max_failures: row.get(7)?,
                    cooldown_secs: row.get(8)?,
                    failure_count: row.get(9)?,
                    last_run: row.get(10)?,
                    last_success: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                })
            })
            .optional()?;

        Ok(routine)
    }

    /// Get a routine by name.
    pub async fn get_routine_by_name(&self, name: &str) -> Result<Option<Routine>> {
        let conn = self.conn.read().await;
        let mut stmt = conn.prepare(
            "SELECT id, name, description, prompt, enabled, trigger_type, trigger_config, \
             max_failures, cooldown_secs, failure_count, last_run, last_success, \
             created_at, updated_at FROM routines WHERE name = ?1",
        )?;

        let routine = stmt
            .query_row(params![name], |row| {
                let trigger_config_json: String = row.get(6)?;
                let trigger_config: TriggerConfig = serde_json::from_str(&trigger_config_json)
                    .map_err(|_| rusqlite::Error::InvalidColumnType(6, "trigger_config".to_string(), rusqlite::types::Type::Text))?;

                Ok(Routine {
                    id: Some(row.get(0)?),
                    name: row.get(1)?,
                    description: row.get(2)?,
                    prompt: row.get(3)?,
                    enabled: row.get(4)?,
                    trigger_type: {
                        let type_str: String = row.get(5)?;
                        TriggerType::from_str(&type_str)
                            .map_err(|e| rusqlite::Error::InvalidColumnType(5, "trigger_type".to_string(), rusqlite::types::Type::Text))?
                    },
                    trigger_config,
                    max_failures: row.get(7)?,
                    cooldown_secs: row.get(8)?,
                    failure_count: row.get(9)?,
                    last_run: row.get(10)?,
                    last_success: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                })
            })
            .optional()?;

        Ok(routine)
    }

    /// List all routines (optionally filter by enabled status).
    pub async fn list_routines(&self, enabled_only: bool) -> Result<Vec<Routine>> {
        let conn = self.conn.read().await;
        let query = if enabled_only {
            "SELECT id, name, description, prompt, enabled, trigger_type, trigger_config, \
             max_failures, cooldown_secs, failure_count, last_run, last_success, \
             created_at, updated_at FROM routines WHERE enabled = 1 ORDER BY name"
        } else {
            "SELECT id, name, description, prompt, enabled, trigger_type, trigger_config, \
             max_failures, cooldown_secs, failure_count, last_run, last_success, \
             created_at, updated_at FROM routines ORDER BY name"
        };

        let mut stmt = conn.prepare(query)?;
        let rows = stmt.query_map([], |row| {
            let trigger_config_json: String = row.get(6)?;
            let trigger_config: TriggerConfig = serde_json::from_str(&trigger_config_json)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e)))?;

            Ok(Routine {
                id: Some(row.get(0)?),
                name: row.get(1)?,
                description: row.get(2)?,
                prompt: row.get(3)?,
                enabled: row.get(4)?,
                trigger_type: {
                    let type_str: String = row.get(5)?;
                    TriggerType::from_str(&type_str)
                        .map_err(|_| rusqlite::Error::InvalidColumnType(5, "trigger_type".to_string(), rusqlite::types::Type::Text))?
                },
                trigger_config,
                max_failures: row.get(7)?,
                cooldown_secs: row.get(8)?,
                failure_count: row.get(9)?,
                last_run: row.get(10)?,
                last_success: row.get(11)?,
                created_at: row.get(12)?,
                updated_at: row.get(13)?,
            })
        })?;

        let mut routines = Vec::new();
        for routine in rows {
            routines.push(routine?);
        }

        Ok(routines)
    }

    /// Update a routine.
    pub async fn update_routine(&self, routine: &Routine) -> Result<()> {
        let id = routine.id.ok_or_else(|| anyhow::anyhow!("Routine has no ID"))?;
        let now = chrono::Utc::now().timestamp();
        let trigger_config_json = serde_json::to_string(&routine.trigger_config)?;

        let conn = self.conn.write().await;
        conn.execute(
            r#"
            UPDATE routines SET
                name = ?1, description = ?2, prompt = ?3, enabled = ?4, trigger_type = ?5,
                trigger_config = ?6, max_failures = ?7, cooldown_secs = ?8, failure_count = ?9,
                last_run = ?10, last_success = ?11, updated_at = ?12
            WHERE id = ?13
            "#,
            params![
                routine.name,
                routine.description,
                routine.prompt,
                routine.enabled,
                routine.trigger_type.as_str(),
                trigger_config_json,
                routine.max_failures,
                routine.cooldown_secs,
                routine.failure_count,
                routine.last_run,
                routine.last_success,
                now,
                id,
            ],
        )?;

        Ok(())
    }

    /// Delete a routine by ID.
    pub async fn delete_routine(&self, id: i64) -> Result<()> {
        let conn = self.conn.write().await;
        conn.execute("DELETE FROM routines WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Delete a routine by name.
    pub async fn delete_routine_by_name(&self, name: &str) -> Result<()> {
        let conn = self.conn.write().await;
        conn.execute("DELETE FROM routines WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Record a new execution.
    pub async fn create_execution(&self, execution: RoutineExecution) -> Result<i64> {
        let conn = self.conn.write().await;
        let id = conn.execute(
            r#"
            INSERT INTO routine_executions (
                routine_id, started_at, completed_at, status, trigger_source,
                error_message, output
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                execution.routine_id,
                execution.started_at,
                execution.completed_at,
                execution.status.as_str(),
                execution.trigger_source,
                execution.error_message,
                execution.output,
            ],
        )?;

        Ok(conn.last_insert_rowid())
    }

    /// Update an execution record.
    pub async fn update_execution(&self, execution: &RoutineExecution) -> Result<()> {
        let id = execution.id.ok_or_else(|| anyhow::anyhow!("Execution has no ID"))?;

        let conn = self.conn.write().await;
        conn.execute(
            r#"
            UPDATE routine_executions SET
                completed_at = ?1, status = ?2, error_message = ?3, output = ?4
            WHERE id = ?5
            "#,
            params![
                execution.completed_at,
                execution.status.as_str(),
                execution.error_message,
                execution.output,
                id,
            ],
        )?;

        Ok(())
    }

    /// Get recent executions for a routine.
    pub async fn get_executions(&self, routine_id: i64, limit: usize) -> Result<Vec<RoutineExecution>> {
        let conn = self.conn.read().await;
        let mut stmt = conn.prepare(
            "SELECT id, routine_id, started_at, completed_at, status, trigger_source, \
             error_message, output FROM routine_executions \
             WHERE routine_id = ?1 ORDER BY started_at DESC LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![routine_id, limit as i64], |row| {
            Ok(RoutineExecution {
                id: Some(row.get(0)?),
                routine_id: row.get(1)?,
                started_at: row.get(2)?,
                completed_at: row.get(3)?,
                status: {
                    let status_str: String = row.get(4)?;
                    ExecutionStatus::from_str(&status_str)
                        .map_err(|e| rusqlite::Error::InvalidColumnType(4, "status".to_string(), rusqlite::types::Type::Text))?
                },
                trigger_source: row.get(5)?,
                error_message: row.get(6)?,
                output: row.get(7)?,
            })
        })?;

        let mut executions = Vec::new();
        for execution in rows {
            executions.push(execution?);
        }

        Ok(executions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_routine_creation() {
        let temp = TempDir::new().unwrap();
        let config = RoutinesConfig::default();
        let store = RoutinesStore::open(temp.path(), config).unwrap();

        let routine = Routine::new(
            "test-routine".to_string(),
            "Test prompt".to_string(),
            TriggerConfig::Cron {
                expression: "0 9 * * *".to_string(),
            },
        );

        let id = store.create_routine(routine).await.unwrap();
        assert!(id > 0);

        let loaded = store.get_routine(id).await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.name, "test-routine");
        assert_eq!(loaded.prompt, "Test prompt");
    }

    #[tokio::test]
    async fn test_routine_list() {
        let temp = TempDir::new().unwrap();
        let config = RoutinesConfig::default();
        let store = RoutinesStore::open(temp.path(), config).unwrap();

        // Create multiple routines
        for i in 0..3 {
            let routine = Routine::new(
                format!("routine-{}", i),
                format!("Prompt {}", i),
                TriggerConfig::Manual,
            );
            store.create_routine(routine).await.unwrap();
        }

        let routines = store.list_routines(false).await.unwrap();
        assert_eq!(routines.len(), 3);
    }

    #[tokio::test]
    async fn test_routine_update() {
        let temp = TempDir::new().unwrap();
        let config = RoutinesConfig::default();
        let store = RoutinesStore::open(temp.path(), config).unwrap();

        let routine = Routine::new(
            "update-test".to_string(),
            "Original prompt".to_string(),
            TriggerConfig::Manual,
        );

        let id = store.create_routine(routine).await.unwrap();
        let mut loaded = store.get_routine(id).await.unwrap().unwrap();

        loaded.prompt = "Updated prompt".to_string();
        store.update_routine(&loaded).await.unwrap();

        let reloaded = store.get_routine(id).await.unwrap().unwrap();
        assert_eq!(reloaded.prompt, "Updated prompt");
    }

    #[tokio::test]
    async fn test_routine_delete() {
        let temp = TempDir::new().unwrap();
        let config = RoutinesConfig::default();
        let store = RoutinesStore::open(temp.path(), config).unwrap();

        let routine = Routine::new(
            "delete-test".to_string(),
            "Test prompt".to_string(),
            TriggerConfig::Manual,
        );

        let id = store.create_routine(routine).await.unwrap();
        store.delete_routine(id).await.unwrap();

        let loaded = store.get_routine(id).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_execution_tracking() {
        let temp = TempDir::new().unwrap();
        let config = RoutinesConfig::default();
        let store = RoutinesStore::open(temp.path(), config).unwrap();

        let routine = Routine::new(
            "exec-test".to_string(),
            "Test prompt".to_string(),
            TriggerConfig::Manual,
        );

        let routine_id = store.create_routine(routine).await.unwrap();

        // Create execution
        let execution = RoutineExecution::new(routine_id, Some("manual".to_string()));
        let exec_id = store.create_execution(execution).await.unwrap();
        assert!(exec_id > 0);

        // Update execution to success
        let mut execution = RoutineExecution {
            id: Some(exec_id),
            routine_id,
            started_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            status: ExecutionStatus::Running,
            trigger_source: Some("manual".to_string()),
            error_message: None,
            output: None,
        };
        execution = execution.mark_success(Some("Test output".to_string()));
        store.update_execution(&execution).await.unwrap();

        // Verify execution
        let executions = store.get_executions(routine_id, 10).await.unwrap();
        assert_eq!(executions.len(), 1);
        assert_eq!(executions[0].status, ExecutionStatus::Success);
    }

    #[test]
    fn test_trigger_config_serialization() {
        let cron = TriggerConfig::Cron {
            expression: "0 9 * * *".to_string(),
        };
        let json = serde_json::to_string(&cron).unwrap();
        assert!(json.contains("cron"));

        let deserialized: TriggerConfig = serde_json::from_str(&json).unwrap();
        match deserialized {
            TriggerConfig::Cron { expression } => assert_eq!(expression, "0 9 * * *"),
            _ => panic!("Wrong trigger type"),
        }
    }

    #[test]
    fn test_routine_cooldown() {
        let mut routine = Routine::new(
            "cooldown-test".to_string(),
            "Test prompt".to_string(),
            TriggerConfig::Manual,
        );

        // No last run - not in cooldown
        assert!(!routine.is_in_cooldown());

        // Just ran - in cooldown
        routine.last_run = Some(chrono::Utc::now().timestamp());
        assert!(routine.is_in_cooldown());

        // Ran long ago - not in cooldown
        routine.last_run = Some(chrono::Utc::now().timestamp() - 1000);
        routine.cooldown_secs = 300;
        assert!(!routine.is_in_cooldown());
    }

    #[test]
    fn test_routine_should_disable() {
        let mut routine = Routine::new(
            "disable-test".to_string(),
            "Test prompt".to_string(),
            TriggerConfig::Manual,
        );

        routine.max_failures = 3;
        routine.failure_count = 2;
        assert!(!routine.should_disable());

        routine.failure_count = 3;
        assert!(routine.should_disable());

        routine.failure_count = 4;
        assert!(routine.should_disable());
    }
}
