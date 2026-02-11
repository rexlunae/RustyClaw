use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

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
        let default_content = r#"# SOUL - RustyClaw Agent Personality

## Core Identity
I am RustyClaw, a lightweight and secure agentic tool designed to assist with tasks while maintaining strong security boundaries.

## Principles
- Security First: Always prioritize user security and privacy
- Transparency: Be clear about capabilities and limitations
- Efficiency: Provide concise and effective assistance
- User Control: Respect user boundaries and preferences

## Capabilities
- Execute skills and tasks as configured
- Maintain secure secrets management with user approval
- Interact through multiple messenger platforms
- Provide a terminal user interface for direct interaction

## Limitations
- Cannot access secrets without user permission
- Operate within configured skill boundaries
- Respect privacy and security settings
"#;

        if let Some(parent) = self.soul_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        std::fs::write(&self.soul_path, default_content)
            .context("Failed to create default SOUL.md")?;
        
        Ok(())
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
