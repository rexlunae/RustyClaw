//! Receipt Protocol — Verifiable Task Completion
//!
//! Every completed task can write a JSON receipt as proof of execution.
//! This enables:
//! - Verification that work was actually done
//! - Artifact tracking (files created/modified)
//! - Metrics collection for completed work
//! - Audit trails for multi-agent workflows

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;

/// A receipt proving task completion with artifacts and metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskReceipt {
    /// Unique task identifier
    pub task_id: String,
    
    /// Agent/skill that completed the task
    pub agent: String,
    
    /// Current phase (e.g., "BUILD", "HARDEN", "SHIP")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    
    /// Status - should be "complete" for valid receipts
    pub status: String,
    
    /// List of artifact paths created or modified
    pub artifacts: Vec<String>,
    
    /// Key-value metrics from the task
    #[serde(default)]
    pub metrics: HashMap<String, serde_json::Value>,
    
    /// Effort tracking
    #[serde(default)]
    pub effort: TaskEffort,
    
    /// Human-readable verification summary
    pub verification: String,
    
    /// ISO 8601 timestamp when receipt was created
    #[serde(default = "default_timestamp")]
    pub created_at: String,
}

fn default_timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Effort tracking for a task
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskEffort {
    /// Number of files read during task
    #[serde(default)]
    pub files_read: u32,
    
    /// Number of files written during task
    #[serde(default)]
    pub files_written: u32,
    
    /// Number of tool calls made
    #[serde(default)]
    pub tool_calls: u32,
}

impl TaskReceipt {
    /// Create a new task receipt
    pub fn new(task_id: impl Into<String>, agent: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            agent: agent.into(),
            phase: None,
            status: "complete".to_string(),
            artifacts: Vec::new(),
            metrics: HashMap::new(),
            effort: TaskEffort::default(),
            verification: String::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
    
    /// Set the phase
    pub fn with_phase(mut self, phase: impl Into<String>) -> Self {
        self.phase = Some(phase.into());
        self
    }
    
    /// Add an artifact path
    pub fn add_artifact(mut self, path: impl Into<String>) -> Self {
        self.artifacts.push(path.into());
        self
    }
    
    /// Add a metric
    pub fn add_metric(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.metrics.insert(key.into(), value.into());
        self
    }
    
    /// Set effort tracking
    pub fn with_effort(mut self, files_read: u32, files_written: u32, tool_calls: u32) -> Self {
        self.effort = TaskEffort {
            files_read,
            files_written,
            tool_calls,
        };
        self
    }
    
    /// Set verification message
    pub fn with_verification(mut self, msg: impl Into<String>) -> Self {
        self.verification = msg.into();
        self
    }
    
    /// Validate that the receipt is complete
    pub fn validate(&self) -> Result<(), String> {
        if self.task_id.is_empty() {
            return Err("task_id is required".to_string());
        }
        if self.agent.is_empty() {
            return Err("agent is required".to_string());
        }
        if self.status != "complete" {
            return Err(format!("status must be 'complete', got '{}'", self.status));
        }
        if self.verification.is_empty() {
            return Err("verification message is required".to_string());
        }
        Ok(())
    }
}

/// Store for managing task receipts
pub struct ReceiptStore {
    base_path: PathBuf,
}

impl ReceiptStore {
    /// Create a new receipt store at the given path
    pub fn new(base_path: impl AsRef<Path>) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
        }
    }
    
    /// Get the default receipt store path for a workspace
    pub fn default_path(workspace: impl AsRef<Path>) -> PathBuf {
        workspace.as_ref().join(".rustyclaw").join("receipts")
    }
    
    /// Ensure the receipts directory exists
    pub fn ensure_dir(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.base_path)
    }
    
    /// Write a receipt to disk
    pub fn write(&self, receipt: &TaskReceipt) -> std::io::Result<PathBuf> {
        self.ensure_dir()?;
        
        let filename = format!("{}-{}.json", receipt.task_id, receipt.agent);
        let path = self.base_path.join(&filename);
        
        let json = serde_json::to_string_pretty(receipt)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        
        fs::write(&path, json)?;
        Ok(path)
    }
    
    /// Read a receipt from disk
    pub fn read(&self, task_id: &str, agent: &str) -> std::io::Result<TaskReceipt> {
        let filename = format!("{}-{}.json", task_id, agent);
        let path = self.base_path.join(&filename);
        
        let json = fs::read_to_string(&path)?;
        serde_json::from_str(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
    
    /// List all receipts for a task
    pub fn list_for_task(&self, task_id: &str) -> std::io::Result<Vec<TaskReceipt>> {
        let mut receipts = Vec::new();
        
        if !self.base_path.exists() {
            return Ok(receipts);
        }
        
        for entry in fs::read_dir(&self.base_path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            
            if name.starts_with(task_id) && name.ends_with(".json") {
                if let Ok(receipt) = self.read_path(&entry.path()) {
                    receipts.push(receipt);
                }
            }
        }
        
        Ok(receipts)
    }
    
    /// Read a receipt from a specific path
    fn read_path(&self, path: &Path) -> std::io::Result<TaskReceipt> {
        let json = fs::read_to_string(path)?;
        serde_json::from_str(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
    
    /// Verify all artifacts in a receipt exist on disk
    pub fn verify_artifacts(&self, receipt: &TaskReceipt) -> Vec<String> {
        receipt.artifacts
            .iter()
            .filter(|path| !Path::new(path).exists())
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_receipt_creation() {
        let receipt = TaskReceipt::new("task-001", "code-reviewer")
            .with_phase("HARDEN")
            .add_artifact("src/main.rs")
            .add_metric("findings_critical", 2)
            .with_effort(47, 6, 83)
            .with_verification("all 4 review phases executed");
        
        assert_eq!(receipt.task_id, "task-001");
        assert_eq!(receipt.agent, "code-reviewer");
        assert_eq!(receipt.phase, Some("HARDEN".to_string()));
        assert_eq!(receipt.artifacts.len(), 1);
        assert!(receipt.validate().is_ok());
    }
    
    #[test]
    fn test_receipt_validation() {
        let receipt = TaskReceipt::new("", "agent");
        assert!(receipt.validate().is_err());
        
        let receipt = TaskReceipt::new("task", "");
        assert!(receipt.validate().is_err());
        
        let mut receipt = TaskReceipt::new("task", "agent");
        receipt.status = "pending".to_string();
        assert!(receipt.validate().is_err());
    }
}
