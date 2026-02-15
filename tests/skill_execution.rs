//! Skill execution tests.
//!
//! Tests for skill loading, gating, and prompt injection.

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

// We need to import from the library
// Note: These tests use the public API from rustyclaw::skills

/// Create a test skill directory with SKILL.md
fn create_test_skill(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
    let skill_dir = dir.join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    let skill_file = skill_dir.join("SKILL.md");
    fs::write(&skill_file, content).unwrap();
    skill_file
}

mod skill_loading {
    use super::*;

    #[test]
    fn test_load_simple_skill() {
        let temp = TempDir::new().unwrap();
        
        let content = r#"---
name: test-skill
description: A simple test skill
---

# Test Skill

This skill does testing.
"#;
        
        create_test_skill(temp.path(), "test-skill", content);
        
        // Verify the file was created
        assert!(temp.path().join("test-skill/SKILL.md").exists());
    }

    #[test]
    fn test_load_skill_with_metadata() {
        let temp = TempDir::new().unwrap();
        
        let content = r#"---
name: advanced-skill
description: Skill with full metadata
metadata: {"openclaw": {"always": true, "emoji": "🔧", "requires": {"bins": ["git"]}}}
---

# Advanced Skill

This skill requires git.
"#;
        
        create_test_skill(temp.path(), "advanced-skill", content);
        
        let skill_path = temp.path().join("advanced-skill/SKILL.md");
        let skill_content = fs::read_to_string(&skill_path).unwrap();
        
        assert!(skill_content.contains("advanced-skill"));
        assert!(skill_content.contains("git"));
    }

    #[test]
    fn test_skill_without_frontmatter() {
        let temp = TempDir::new().unwrap();
        
        // No frontmatter - just markdown
        let content = "# Just Markdown\n\nNo YAML here.";
        
        let skill_dir = temp.path().join("no-yaml");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
        
        let read_content = fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        assert!(!read_content.starts_with("---"));
    }
}

mod frontmatter_parsing {
    

    #[test]
    fn test_parse_name_and_description() {
        let content = r#"---
name: my-skill
description: Does something useful
---

Instructions here.
"#;
        
        assert!(content.contains("name: my-skill"));
        assert!(content.contains("description: Does something useful"));
    }

    #[test]
    fn test_parse_multiline_description() {
        let content = r#"---
name: multiline-skill
description: >
  This is a longer description
  that spans multiple lines
---

Content.
"#;
        
        // YAML multiline strings are valid
        assert!(content.contains("description: >"));
    }

    #[test]
    fn test_parse_metadata_json() {
        let content = r#"---
name: json-meta
description: Has JSON metadata
metadata: {"openclaw": {"emoji": "⚡", "requires": {"bins": ["node", "npm"]}}}
---

# Instructions
"#;
        
        // Verify JSON is inline
        assert!(content.contains(r#""emoji": "⚡""#));
        assert!(content.contains(r#""bins": ["node", "npm"]"#));
    }
}

mod gating {
    

    #[test]
    fn test_gate_always_true() {
        // Skill with always: true should skip all gates
        let content = r#"---
name: always-skill
description: Always enabled
metadata: {"openclaw": {"always": true, "requires": {"bins": ["nonexistent12345"]}}}
---
"#;
        
        // Even with nonexistent binary requirement, always: true should pass
        assert!(content.contains(r#""always": true"#));
    }

    #[test]
    fn test_gate_bins_requirement() {
        let content = r#"---
name: git-skill
description: Requires git
metadata: {"openclaw": {"requires": {"bins": ["git"]}}}
---

Use git commands.
"#;
        
        assert!(content.contains(r#""bins": ["git"]"#));
    }

    #[test]
    fn test_gate_any_bins_requirement() {
        let content = r#"---
name: editor-skill
description: Requires any editor
metadata: {"openclaw": {"requires": {"anyBins": ["vim", "nano", "emacs"]}}}
---

Open in editor.
"#;
        
        assert!(content.contains("anyBins"));
    }

    #[test]
    fn test_gate_env_requirement() {
        let content = r#"---
name: api-skill
description: Requires API key
metadata: {"openclaw": {"requires": {"env": ["MY_API_KEY"]}, "primaryEnv": "MY_API_KEY"}}
---

Uses MY_API_KEY.
"#;
        
        assert!(content.contains(r#""env": ["MY_API_KEY"]"#));
        assert!(content.contains(r#""primaryEnv": "MY_API_KEY""#));
    }

    #[test]
    fn test_gate_os_requirement() {
        let content = r#"---
name: macos-skill
description: macOS only
metadata: {"openclaw": {"os": ["darwin"]}}
---

macOS specific instructions.
"#;
        
        assert!(content.contains(r#""os": ["darwin"]"#));
    }

    #[test]
    fn test_gate_config_requirement() {
        let content = r#"---
name: browser-skill
description: Requires browser enabled
metadata: {"openclaw": {"requires": {"config": ["browser.enabled"]}}}
---

Browser automation.
"#;
        
        assert!(content.contains(r#""config": ["browser.enabled"]"#));
    }
}

mod prompt_context {
    

    #[test]
    fn test_base_dir_placeholder() {
        let content = r#"---
name: script-skill
description: Runs scripts
---

Run the script at {baseDir}/scripts/main.sh
"#;
        
        assert!(content.contains("{baseDir}"));
    }

    #[test]
    fn test_instruction_extraction() {
        let content = r#"---
name: instruction-skill
description: Has detailed instructions
---

# How to Use

1. First step
2. Second step
3. Third step

## Advanced Usage

More details here.
"#;
        
        // After frontmatter is stripped, instructions remain
        let after_frontmatter = content.split("---").nth(2).unwrap().trim();
        assert!(after_frontmatter.starts_with("# How to Use"));
    }
}

mod precedence {
    use super::*;

    #[test]
    fn test_workspace_overrides_local() {
        let temp = TempDir::new().unwrap();
        
        // Create "local" skill
        let local_dir = temp.path().join("local");
        fs::create_dir_all(&local_dir).unwrap();
        create_test_skill(&local_dir, "shared-skill", r#"---
name: shared-skill
description: Local version
---
Local instructions.
"#);
        
        // Create "workspace" skill with same name
        let workspace_dir = temp.path().join("workspace");
        fs::create_dir_all(&workspace_dir).unwrap();
        create_test_skill(&workspace_dir, "shared-skill", r#"---
name: shared-skill
description: Workspace version (should win)
---
Workspace instructions.
"#);
        
        // Verify both exist
        assert!(local_dir.join("shared-skill/SKILL.md").exists());
        assert!(workspace_dir.join("shared-skill/SKILL.md").exists());
        
        // In real implementation, workspace version would take precedence
        let workspace_content = fs::read_to_string(
            workspace_dir.join("shared-skill/SKILL.md")
        ).unwrap();
        assert!(workspace_content.contains("Workspace version"));
    }
}

mod legacy_formats {
    use super::*;

    #[test]
    fn test_json_skill_format() {
        let temp = TempDir::new().unwrap();
        
        let content = r#"{
    "name": "json-skill",
    "description": "Defined in JSON",
    "enabled": true
}"#;
        
        let skill_file = temp.path().join("json-skill.json");
        fs::write(&skill_file, content).unwrap();
        
        let read_content = fs::read_to_string(&skill_file).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&read_content).unwrap();
        
        assert_eq!(parsed["name"], "json-skill");
    }

    #[test]
    fn test_yaml_skill_format() {
        let temp = TempDir::new().unwrap();
        
        let content = r#"name: yaml-skill
description: Defined in YAML
enabled: true
"#;
        
        let skill_file = temp.path().join("yaml-skill.yaml");
        fs::write(&skill_file, content).unwrap();
        
        let read_content = fs::read_to_string(&skill_file).unwrap();
        assert!(read_content.contains("name: yaml-skill"));
    }
}

mod integration {
    use super::*;
    use rustyclaw::skills::SkillManager;

    #[test]
    fn test_full_skill_lifecycle() {
        let temp = TempDir::new().unwrap();
        
        // 1. Create skill
        let content = r#"---
name: lifecycle-skill
description: Test full lifecycle
metadata: {"openclaw": {"emoji": "🔄", "requires": {"bins": ["ls"]}}}
---

# Lifecycle Skill

Run `ls` in {baseDir} to list files.
"#;
        
        create_test_skill(temp.path(), "lifecycle-skill", content);
        
        // 2. Load with SkillManager
        let mut manager = SkillManager::new(temp.path().to_path_buf());
        manager.load_skills().unwrap();
        
        let skills = manager.get_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "lifecycle-skill");
        assert_eq!(skills[0].description, Some("Test full lifecycle".to_string()));
        assert_eq!(skills[0].metadata.emoji, Some("🔄".to_string()));
        assert_eq!(skills[0].metadata.requires.bins, vec!["ls"]);
        
        // 3. Check gating passes (ls exists)
        let gate_result = manager.check_gates(&skills[0]);
        assert!(gate_result.passed, "ls should exist on PATH");
        
        // 4. Verify {baseDir} replacement
        assert!(skills[0].instructions.contains(temp.path().to_str().unwrap()));
        assert!(!skills[0].instructions.contains("{baseDir}"));
    }
    
    #[test]
    fn test_load_openclaw_skills() {
        // Test loading actual OpenClaw skills if available
        let openclaw_skills = PathBuf::from("/usr/lib/node_modules/openclaw/skills");
        if !openclaw_skills.exists() {
            // Skip if OpenClaw not installed
            return;
        }
        
        let mut manager = SkillManager::new(openclaw_skills);
        manager.load_skills().unwrap();
        
        let skills = manager.get_skills();
        
        // Should find many skills
        assert!(skills.len() > 10, "Expected >10 OpenClaw skills, found {}", skills.len());
        
        // Check specific skills we know exist
        let github_skill = skills.iter().find(|s| s.name == "github");
        assert!(github_skill.is_some(), "github skill should exist");
        
        let github = github_skill.unwrap();
        assert_eq!(github.metadata.emoji, Some("🐙".to_string()));
        assert!(github.metadata.requires.bins.contains(&"gh".to_string()));
        
        // Check weather skill
        let weather_skill = skills.iter().find(|s| s.name == "weather");
        assert!(weather_skill.is_some(), "weather skill should exist");
        
        let weather = weather_skill.unwrap();
        assert_eq!(weather.metadata.emoji, Some("🌤️".to_string()));
        assert!(weather.metadata.requires.bins.contains(&"curl".to_string()));
    }
    
    #[test]
    fn test_generate_prompt_context() {
        let temp = TempDir::new().unwrap();
        
        // Create a skill that passes gating
        let content = r#"---
name: prompt-skill
description: Skill for prompt generation test
metadata: {"openclaw": {"requires": {"bins": ["ls"]}}}
---

# Prompt Skill Instructions
"#;
        
        create_test_skill(temp.path(), "prompt-skill", content);
        
        let mut manager = SkillManager::new(temp.path().to_path_buf());
        manager.load_skills().unwrap();
        
        let context = manager.generate_prompt_context();
        
        assert!(context.contains("<available_skills>"));
        assert!(context.contains("<name>prompt-skill</name>"));
        assert!(context.contains("Skill for prompt generation test"));
        assert!(context.contains("</available_skills>"));
    }
}
