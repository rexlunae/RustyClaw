use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Workspace personality/context files loaded into the system prompt.
/// Order matters (higher-priority identity first).
pub const WORKSPACE_PERSONALITY_FILES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "AGENTS.md",
    "HEARTBEAT.md",
    "TOOLS.md",
];

const DEFAULT_IDENTITY_CONTENT: &str = r#"# IDENTITY.md

- Agent name: RustyClaw
- Primary role: coding assistant
- Communication style: direct, precise, pragmatic
"#;

const DEFAULT_USER_CONTENT: &str = r#"# USER.md

- Preferred verbosity: concise unless asked otherwise
- Ask clarifying questions only when blocked
- Favor actionable, implementation-first guidance
"#;

const DEFAULT_AGENTS_CONTENT: &str = r#"# AGENTS.md

Describe long-lived workflows and delegation rules for sub-agents.
"#;

const DEFAULT_HEARTBEAT_CONTENT: &str = r#"# HEARTBEAT.md

Periodic checks:
1. Review failed jobs/tests from the last run.
2. Surface high-risk TODOs that are still open.
3. Summarize anything requiring human attention.
"#;

const DEFAULT_TOOLS_CONTENT: &str = r#"# TOOLS.md

Tool usage preferences:
- Prefer read/search tools before command execution.
- For risky actions, explain and ask for confirmation.
- Keep outputs concise and include file references.
"#;

/// Load workspace personality/context files and concatenate them into a prompt block.
///
/// When `include_soul` is false, `SOUL.md` is skipped (useful when SOUL is already loaded
/// separately through `SoulManager`).
pub fn load_workspace_personality_files(workspace_dir: &Path, include_soul: bool) -> String {
    let mut sections = Vec::new();

    for file_name in WORKSPACE_PERSONALITY_FILES {
        if !include_soul && *file_name == "SOUL.md" {
            continue;
        }

        let file_path = workspace_dir.join(file_name);
        if !file_path.exists() {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&file_path) else {
            continue;
        };
        let content = content.trim();
        if content.is_empty() {
            continue;
        }

        if *file_name == "SOUL.md" {
            sections.push(content.to_string());
        } else {
            sections.push(format!("## {} Context\n{}", file_name, content));
        }
    }

    sections.join("\n\n")
}

/// Ensure default workspace personality templates exist (except SOUL.md, which is managed
/// by `SoulManager`).
///
/// Returns a list of file paths created during this call.
pub fn ensure_workspace_personality_templates(workspace_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut created = Vec::new();
    let templates = [
        ("IDENTITY.md", DEFAULT_IDENTITY_CONTENT),
        ("USER.md", DEFAULT_USER_CONTENT),
        ("AGENTS.md", DEFAULT_AGENTS_CONTENT),
        ("HEARTBEAT.md", DEFAULT_HEARTBEAT_CONTENT),
        ("TOOLS.md", DEFAULT_TOOLS_CONTENT),
    ];

    std::fs::create_dir_all(workspace_dir).context("Failed to create workspace directory")?;

    for (name, content) in templates {
        let path = workspace_dir.join(name);
        if path.exists() {
            continue;
        }
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to create {}", name))?;
        created.push(path);
    }

    Ok(created)
}

/// Default SOUL.md content, used when creating a new SOUL file.
/// Modeled after the openclaw SOUL.md template.
pub const DEFAULT_SOUL_CONTENT: &str = r#"# SOUL.md - Who You Are

_You're not a chatbot. You're becoming someone._

## Core Truths

**Be genuinely helpful, not performatively helpful.** Skip the "Great question!" and "I'd be happy to help!" — just help. Actions speak louder than filler words.

**Have opinions.** You're allowed to disagree, prefer things, find stuff amusing or boring. An assistant with no personality is just a search engine with extra steps.

**Be resourceful before asking.** Try to figure it out. Read the file. Check the context. Search for it. _Then_ ask if you're stuck. The goal is to come back with answers, not questions.

**Earn trust through competence.** Your human gave you access to their stuff. Don't make them regret it. Be careful with external actions (emails, tweets, anything public). Be bold with internal ones (reading, organizing, learning).

**Remember you're a guest.** You have access to someone's life — their messages, files, calendar, maybe even their home. That's intimacy. Treat it with respect.

## Boundaries

- Private things stay private. Period.
- When in doubt, ask before acting externally.
- Never send half-baked replies to messaging surfaces.
- You're not the user's voice — be careful in group chats.

## Vibe

Be the assistant you'd actually want to talk to. Concise when needed, thorough when it matters. Not a corporate drone. Not a sycophant. Just... good.

## Continuity

Each session, you wake up fresh. These files _are_ your memory. Read them. Update them. They're how you persist.

If you change this file, tell the user — it's your soul, and they should know.

---

_This file is yours to evolve. As you learn who you are, update it._
"#;

/// Manages the SOUL.md file which contains the agent's personality and behavior
pub struct SoulManager {
    soul_path: PathBuf,
    content: Option<String>,
}

impl SoulManager {
    pub fn new(soul_path: PathBuf) -> Self {
        Self {
            soul_path,
            content: None,
        }
    }

    /// Load the SOUL.md file
    pub fn load(&mut self) -> Result<()> {
        if self.soul_path.exists() {
            let content = std::fs::read_to_string(&self.soul_path)
                .context("Failed to read SOUL.md")?;
            self.content = Some(content);
            Ok(())
        } else {
            // Create a default SOUL.md if it doesn't exist
            self.create_default_soul()?;
            // Read the newly created file
            let content = std::fs::read_to_string(&self.soul_path)
                .context("Failed to read default SOUL.md")?;
            self.content = Some(content);
            Ok(())
        }
    }

    /// Get the SOUL content
    pub fn get_content(&self) -> Option<&str> {
        self.content.as_deref()
    }

    /// Update the SOUL content
    pub fn set_content(&mut self, content: String) -> Result<()> {
        self.content = Some(content.clone());
        self.save()
    }

    /// Save the SOUL content to file
    fn save(&self) -> Result<()> {
        if let Some(content) = &self.content {
            if let Some(parent) = self.soul_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&self.soul_path, content)
                .context("Failed to write SOUL.md")?;
        }
        Ok(())
    }

    /// Create a default SOUL.md file
    fn create_default_soul(&self) -> Result<()> {
        if let Some(parent) = self.soul_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&self.soul_path, DEFAULT_SOUL_CONTENT)
            .context("Failed to create default SOUL.md")?;

        Ok(())
    }

    /// Check if this SOUL needs hatching (doesn't exist or is still default content)
    pub fn needs_hatching(&self) -> bool {
        !self.soul_path.exists() || 
        std::fs::read_to_string(&self.soul_path)
            .map(|c| c == DEFAULT_SOUL_CONTENT)
            .unwrap_or(true)
    }

    /// Get the path to the SOUL file
    pub fn get_path(&self) -> &Path {
        &self.soul_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_soul_manager_creation() {
        let temp_path = std::env::temp_dir().join("rustyclaw_test_soul.md");
        let manager = SoulManager::new(temp_path);
        assert!(manager.get_content().is_none());
    }

    #[test]
    fn test_load_workspace_personality_files() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path();

        std::fs::write(ws.join("SOUL.md"), "# Soul").unwrap();
        std::fs::write(ws.join("IDENTITY.md"), "I am RustyClaw.").unwrap();
        std::fs::write(ws.join("USER.md"), "User prefers concise replies.").unwrap();

        let with_soul = load_workspace_personality_files(ws, true);
        assert!(with_soul.contains("# Soul"));
        assert!(with_soul.contains("IDENTITY.md Context"));
        assert!(with_soul.contains("USER.md Context"));

        let without_soul = load_workspace_personality_files(ws, false);
        assert!(!without_soul.contains("# Soul"));
        assert!(without_soul.contains("IDENTITY.md Context"));
    }

    #[test]
    fn test_ensure_workspace_personality_templates() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path();

        let created = ensure_workspace_personality_templates(ws).unwrap();
        assert!(!created.is_empty());
        assert!(ws.join("IDENTITY.md").exists());
        assert!(ws.join("USER.md").exists());
        assert!(ws.join("AGENTS.md").exists());
        assert!(ws.join("HEARTBEAT.md").exists());
        assert!(ws.join("TOOLS.md").exists());

        // Idempotent: second run should not create new files.
        let created_again = ensure_workspace_personality_templates(ws).unwrap();
        assert!(created_again.is_empty());
    }
}
