//! Workspace context injection.
//!
//! Loads and injects workspace files into system prompts for agent continuity.
//! This enables the agent to maintain personality (SOUL.md), remember context
//! (MEMORY.md), and follow workspace conventions (AGENTS.md, TOOLS.md).

use chrono::{Duration, Local};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Session type for security scoping.
///
/// Different session types have different access levels to sensitive files
/// like MEMORY.md and USER.md which may contain private information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionType {
    /// Main/direct session with the owner.
    /// Has full access to all workspace files including MEMORY.md.
    Main,
    /// Group chat or shared context.
    /// Restricted access — excludes MEMORY.md and USER.md for privacy.
    Group,
    /// Isolated sub-agent session.
    /// May have restricted access depending on spawning context.
    Isolated,
}

/// Configuration for workspace context injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceContextConfig {
    /// Enable workspace file injection.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Inject SOUL.md (personality).
    #[serde(default = "default_true")]
    pub inject_soul: bool,

    /// Inject AGENTS.md (behavior guidelines).
    #[serde(default = "default_true")]
    pub inject_agents: bool,

    /// Inject TOOLS.md (tool usage notes).
    #[serde(default = "default_true")]
    pub inject_tools: bool,

    /// Inject IDENTITY.md (agent identity).
    #[serde(default = "default_true")]
    pub inject_identity: bool,

    /// Inject USER.md (user profile) — main session only.
    #[serde(default = "default_true")]
    pub inject_user: bool,

    /// Inject MEMORY.md (long-term memory) — main session only.
    #[serde(default = "default_true")]
    pub inject_memory: bool,

    /// Inject HEARTBEAT.md (periodic task checklist).
    #[serde(default = "default_true")]
    pub inject_heartbeat: bool,

    /// Inject daily memory files (memory/YYYY-MM-DD.md) — main session only.
    #[serde(default = "default_true")]
    pub inject_daily: bool,

    /// Number of daily memory files to include (today + N days back).
    #[serde(default = "default_daily_lookback")]
    pub daily_lookback_days: u32,
}

fn default_true() -> bool {
    true
}

fn default_daily_lookback() -> u32 {
    1 // Today + yesterday
}

impl Default for WorkspaceContextConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            inject_soul: true,
            inject_agents: true,
            inject_tools: true,
            inject_identity: true,
            inject_user: true,
            inject_memory: true,
            inject_heartbeat: true,
            inject_daily: true,
            daily_lookback_days: default_daily_lookback(),
        }
    }
}

/// Workspace file metadata.
struct WorkspaceFile {
    /// Relative path from workspace.
    path: &'static str,
    /// Header to use in prompt.
    header: &'static str,
    /// Only include in main session (privacy).
    main_only: bool,
    /// Config field to check for inclusion.
    config_field: ConfigField,
}

/// Which config field controls this file.
enum ConfigField {
    Soul,
    Agents,
    Tools,
    Identity,
    User,
    Memory,
    Heartbeat,
}

const WORKSPACE_FILES: &[WorkspaceFile] = &[
    WorkspaceFile {
        path: "SOUL.md",
        header: "SOUL.md",
        main_only: false,
        config_field: ConfigField::Soul,
    },
    WorkspaceFile {
        path: "AGENTS.md",
        header: "AGENTS.md",
        main_only: false,
        config_field: ConfigField::Agents,
    },
    WorkspaceFile {
        path: "TOOLS.md",
        header: "TOOLS.md",
        main_only: false,
        config_field: ConfigField::Tools,
    },
    WorkspaceFile {
        path: "IDENTITY.md",
        header: "IDENTITY.md",
        main_only: false,
        config_field: ConfigField::Identity,
    },
    WorkspaceFile {
        path: "USER.md",
        header: "USER.md",
        main_only: true, // Privacy: only in main session
        config_field: ConfigField::User,
    },
    WorkspaceFile {
        path: "MEMORY.md",
        header: "MEMORY.md",
        main_only: true, // Privacy: only in main session
        config_field: ConfigField::Memory,
    },
    WorkspaceFile {
        path: "HEARTBEAT.md",
        header: "HEARTBEAT.md",
        main_only: false,
        config_field: ConfigField::Heartbeat,
    },
];

/// Sub-agent context with parent information for isolation.
#[derive(Debug, Clone, Default)]
pub struct SubagentInfo {
    /// Parent session key for communication.
    pub parent_key: Option<String>,
    /// Task description assigned to this sub-agent.
    pub task: Option<String>,
    /// Label if provided.
    pub label: Option<String>,
}

/// Workspace context builder.
///
/// Loads workspace files and builds system prompt sections for injection
/// into agent conversations.
pub struct WorkspaceContext {
    workspace_dir: PathBuf,
    config: WorkspaceContextConfig,
    subagent_info: Option<SubagentInfo>,
}

impl WorkspaceContext {
    /// Create a new workspace context builder.
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir,
            config: WorkspaceContextConfig::default(),
            subagent_info: None,
        }
    }

    /// Create a workspace context with custom config.
    pub fn with_config(workspace_dir: PathBuf, config: WorkspaceContextConfig) -> Self {
        Self {
            workspace_dir,
            config,
            subagent_info: None,
        }
    }

    /// Create a workspace context for a sub-agent session.
    pub fn for_subagent(
        workspace_dir: PathBuf,
        config: WorkspaceContextConfig,
        info: SubagentInfo,
    ) -> Self {
        Self {
            workspace_dir,
            config,
            subagent_info: Some(info),
        }
    }

    /// Check if a file should be included based on config and session type.
    fn should_include(&self, file: &WorkspaceFile, session_type: SessionType) -> bool {
        // Skip main-only files in non-main sessions
        if file.main_only && session_type != SessionType::Main {
            return false;
        }

        // Check config field
        match file.config_field {
            ConfigField::Soul => self.config.inject_soul,
            ConfigField::Agents => self.config.inject_agents,
            ConfigField::Tools => self.config.inject_tools,
            ConfigField::Identity => self.config.inject_identity,
            ConfigField::User => self.config.inject_user,
            ConfigField::Memory => self.config.inject_memory,
            ConfigField::Heartbeat => self.config.inject_heartbeat,
        }
    }

    /// Build system prompt section from workspace files.
    ///
    /// Returns a formatted string containing all applicable workspace files
    /// for inclusion in the system prompt.
    pub fn build_context(&self, session_type: SessionType) -> String {
        if !self.config.enabled {
            return String::new();
        }

        let mut sections = Vec::new();

        // Load standard workspace files
        for file in WORKSPACE_FILES {
            if !self.should_include(file, session_type) {
                continue;
            }

            let path = self.workspace_dir.join(file.path);

            if let Ok(content) = fs::read_to_string(&path) {
                let content = content.trim();
                if !content.is_empty() {
                    sections.push(format!("## {}\n{}", file.header, content));
                }
            }
        }

        // Add daily memory files for main session
        if session_type == SessionType::Main && self.config.inject_daily {
            if let Some(daily) = self.load_daily_memory() {
                sections.push(daily);
            }
        }

        // Add sub-agent guidance for isolated sessions
        if session_type == SessionType::Isolated {
            sections.push(self.build_subagent_guidance());
        }

        if sections.is_empty() {
            String::new()
        } else {
            format!(
                "# Project Context\n\
                 The following project context files have been loaded:\n\n{}",
                sections.join("\n\n---\n\n")
            )
        }
    }

    /// Build guidance section for sub-agent sessions.
    fn build_subagent_guidance(&self) -> String {
        let mut guidance = String::from("## Sub-Agent Guidelines\n\n");
        guidance.push_str(
            "You are running in an **isolated sub-agent session** spawned by a parent agent.\n\n",
        );

        // Add task context if available
        if let Some(ref info) = self.subagent_info {
            if let Some(ref task) = info.task {
                guidance.push_str(&format!("**Your assigned task:** {}\n\n", task));
            }
            if let Some(ref label) = info.label {
                guidance.push_str(&format!("**Session label:** {}\n\n", label));
            }
        }

        guidance.push_str(
"### Communication
- Your final output will be delivered to the parent session automatically when you complete
- If you need to send interim updates, use `sessions_send` with the parent session key
- Do **not** assume access to messaging channels (Signal, Discord, etc.) — route through the parent

### Blocking Issues
If you cannot proceed due to missing resources (e.g., browser not attached, credentials unavailable):
1. **Clearly state what's blocking you** — be specific about the missing resource
2. **List actions needed** — what the user or parent agent can do to unblock you
3. **Exit cleanly** — don't retry indefinitely or loop; complete with a clear status message

Example: \"Browser relay not connected. To proceed, the user needs to attach a Chrome tab via the OpenClaw Browser Relay toolbar button, then re-run this task.\"

### Scope
- Focus on your assigned task; do not take on unrelated work
- You have access to the same tools as the parent, but may lack delivery context
- If the task is complete, summarize your results clearly for the parent session
"
        );

        guidance
    }

    /// Load today's and recent daily memory files.
    fn load_daily_memory(&self) -> Option<String> {
        let today = Local::now().date_naive();
        let mut daily_sections = Vec::new();

        for i in 0..=self.config.daily_lookback_days {
            let date = today - Duration::days(i as i64);
            let filename = format!("memory/{}.md", date.format("%Y-%m-%d"));
            let path = self.workspace_dir.join(&filename);

            if let Ok(content) = fs::read_to_string(&path) {
                let content = content.trim();
                if !content.is_empty() {
                    daily_sections.push(format!("### {}\n{}", filename, content));
                }
            }
        }

        if daily_sections.is_empty() {
            None
        } else {
            Some(format!(
                "## Recent Daily Notes\n{}",
                daily_sections.join("\n\n")
            ))
        }
    }

    /// Get list of files that should be audited on startup.
    ///
    /// Returns a list of (path, exists) tuples for workspace file status.
    pub fn audit_files(&self, session_type: SessionType) -> Vec<(String, bool)> {
        WORKSPACE_FILES
            .iter()
            .filter(|f| self.should_include(f, session_type))
            .map(|f| {
                let exists = self.workspace_dir.join(f.path).exists();
                (f.path.to_string(), exists)
            })
            .collect()
    }

    /// Get the workspace directory.
    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_workspace() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("SOUL.md"), "Be helpful and concise.").unwrap();
        fs::write(dir.path().join("MEMORY.md"), "User prefers Rust.").unwrap();
        fs::write(dir.path().join("AGENTS.md"), "Follow instructions.").unwrap();
        fs::create_dir(dir.path().join("memory")).unwrap();

        let today = Local::now().format("%Y-%m-%d").to_string();
        fs::write(
            dir.path().join(format!("memory/{}.md", today)),
            "# Today\nWorked on RustyClaw.",
        )
        .unwrap();
        dir
    }

    #[test]
    fn test_main_session_includes_memory() {
        let workspace = setup_workspace();
        let ctx = WorkspaceContext::new(workspace.path().to_path_buf());

        let prompt = ctx.build_context(SessionType::Main);
        assert!(prompt.contains("SOUL.md"));
        assert!(prompt.contains("MEMORY.md"));
        assert!(prompt.contains("User prefers Rust"));
    }

    #[test]
    fn test_group_session_excludes_memory() {
        let workspace = setup_workspace();
        let ctx = WorkspaceContext::new(workspace.path().to_path_buf());

        let prompt = ctx.build_context(SessionType::Group);
        assert!(prompt.contains("SOUL.md"));
        assert!(!prompt.contains("MEMORY.md"));
        assert!(!prompt.contains("User prefers Rust"));
    }

    #[test]
    fn test_isolated_session_excludes_memory() {
        let workspace = setup_workspace();
        let ctx = WorkspaceContext::new(workspace.path().to_path_buf());

        let prompt = ctx.build_context(SessionType::Isolated);
        assert!(prompt.contains("SOUL.md"));
        assert!(!prompt.contains("MEMORY.md"));
    }

    #[test]
    fn test_daily_memory_loading() {
        let workspace = setup_workspace();
        let ctx = WorkspaceContext::new(workspace.path().to_path_buf());

        let prompt = ctx.build_context(SessionType::Main);
        assert!(prompt.contains("Recent Daily Notes"));
        assert!(prompt.contains("Worked on RustyClaw"));
    }

    #[test]
    fn test_disabled_context() {
        let workspace = setup_workspace();
        let config = WorkspaceContextConfig {
            enabled: false,
            ..Default::default()
        };
        let ctx = WorkspaceContext::with_config(workspace.path().to_path_buf(), config);

        let prompt = ctx.build_context(SessionType::Main);
        assert!(prompt.is_empty());
    }

    #[test]
    fn test_selective_injection() {
        let workspace = setup_workspace();
        let config = WorkspaceContextConfig {
            enabled: true,
            inject_soul: true,
            inject_memory: false, // Disabled
            ..Default::default()
        };
        let ctx = WorkspaceContext::with_config(workspace.path().to_path_buf(), config);

        let prompt = ctx.build_context(SessionType::Main);
        assert!(prompt.contains("SOUL.md"));
        assert!(!prompt.contains("MEMORY.md"));
    }

    #[test]
    fn test_audit_files() {
        let workspace = setup_workspace();
        let ctx = WorkspaceContext::new(workspace.path().to_path_buf());

        let audit = ctx.audit_files(SessionType::Main);

        // SOUL.md exists
        assert!(audit.iter().any(|(p, e)| p == "SOUL.md" && *e));
        // MEMORY.md exists
        assert!(audit.iter().any(|(p, e)| p == "MEMORY.md" && *e));
        // TOOLS.md doesn't exist
        assert!(audit.iter().any(|(p, e)| p == "TOOLS.md" && !*e));
    }
}
