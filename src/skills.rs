use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Represents a skill that can be loaded and executed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: Option<String>,
    pub path: PathBuf,
    pub enabled: bool,
}

/// Manages skills compatible with OpenClaw
pub struct SkillManager {
    skills_dir: PathBuf,
    skills: Vec<Skill>,
}

impl SkillManager {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            skills: Vec::new(),
        }
    }

    /// Load skills from the skills directory
    pub fn load_skills(&mut self) -> Result<()> {
        if !self.skills_dir.exists() {
            std::fs::create_dir_all(&self.skills_dir)
                .context("Failed to create skills directory")?;
            return Ok(());
        }

        self.skills.clear();

        for entry in std::fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                if let Some(extension) = path.extension() {
                    // Support various skill formats (OpenClaw compatible)
                    if extension == "skill" || extension == "json" || extension == "yaml" || extension == "yml" {
                        if let Ok(skill) = self.load_skill(&path) {
                            self.skills.push(skill);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Load a single skill from a file
    fn load_skill(&self, path: &Path) -> Result<Skill> {
        // Check extension before reading file for efficiency
        let is_json = path.extension().is_some_and(|e| e == "json");
        let is_yaml = path.extension().is_some_and(|e| e == "yaml" || e == "yml");
        
        if !is_json && !is_yaml {
            anyhow::bail!("Unsupported skill file format: {:?}", path);
        }
        
        let content = std::fs::read_to_string(path)?;
        
        // Parse based on extension
        let skill: Skill = if is_json {
            serde_json::from_str(&content)?
        } else {
            serde_yaml::from_str(&content)?
        };

        Ok(skill)
    }

    /// Get all loaded skills
    pub fn get_skills(&self) -> &[Skill] {
        &self.skills
    }

    /// Get a specific skill by name
    pub fn get_skill(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    /// Enable or disable a skill
    pub fn set_skill_enabled(&mut self, name: &str, enabled: bool) -> Result<()> {
        if let Some(skill) = self.skills.iter_mut().find(|s| s.name == name) {
            skill.enabled = enabled;
            Ok(())
        } else {
            anyhow::bail!("Skill not found: {}", name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_manager_creation() {
        let temp_dir = std::env::temp_dir().join("rustyclaw_test_skills");
        let manager = SkillManager::new(temp_dir);
        assert_eq!(manager.get_skills().len(), 0);
    }
}
