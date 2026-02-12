use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

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

    #[test]
    fn test_soul_manager_creation() {
        let temp_path = std::env::temp_dir().join("rustyclaw_test_soul.md");
        let manager = SoulManager::new(temp_path);
        assert!(manager.get_content().is_none());
    }
}
