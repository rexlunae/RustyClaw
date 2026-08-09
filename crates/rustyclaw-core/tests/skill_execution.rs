//! Skill loading, gating and prompt injection, through `SkillManager`.
//!
//! This file used to carry seventeen more tests across `skill_loading`,
//! `frontmatter_parsing`, `gating`, `prompt_context`, `precedence` and
//! `legacy_formats`. Every one of them wrote a `SKILL.md` string and then
//! asserted that the string contained what had just been written — none
//! loaded a skill or called into the crate, so no change to skill handling
//! could have failed them. They were removed rather than enabled: green
//! tests that cannot fail are the misleading coverage signal this directory
//! was reported for, not a cure for it.

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Create a test skill directory with SKILL.md
fn create_test_skill(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
    let skill_dir = dir.join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    let skill_file = skill_dir.join("SKILL.md");
    fs::write(&skill_file, content).unwrap();
    skill_file
}

mod integration {
    use super::*;
    use rustyclaw_core::skills::SkillManager;

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
        assert_eq!(
            skills[0].description,
            Some("Test full lifecycle".to_string())
        );
        assert_eq!(skills[0].metadata.emoji, Some("🔄".to_string()));
        assert_eq!(skills[0].metadata.requires.bins, vec!["ls"]);

        // 3. Check gating passes (ls exists)
        let gate_result = manager.check_gates(&skills[0]);
        assert!(gate_result.passed, "ls should exist on PATH");

        // 4. Verify {baseDir} replacement
        assert!(
            skills[0]
                .instructions
                .contains(temp.path().to_str().unwrap())
        );
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
        assert!(
            skills.len() > 10,
            "Expected >10 OpenClaw skills, found {}",
            skills.len()
        );

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
