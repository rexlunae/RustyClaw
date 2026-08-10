//! Skill manager tests.

#![allow(unused_imports)]
use super::*;

use super::*;

#[test]
fn test_skill_manager_creation() {
    let temp_dir = std::env::temp_dir().join("rustyclaw_test_skills");
    let manager = SkillManager::new(temp_dir);
    assert_eq!(manager.get_skills().len(), 0);
}

#[test]
fn test_parse_frontmatter_with_yaml() {
    let content = r#"---
name: test-skill
description: A test skill
---

# Instructions

Do the thing.
"#;
    let (fm, instructions) = parse_frontmatter(content).unwrap();
    assert_eq!(fm["name"].as_str(), Some("test-skill"));
    assert_eq!(fm["description"].as_str(), Some("A test skill"));
    assert!(instructions.contains("Do the thing"));
}

#[test]
fn test_parse_frontmatter_without_yaml() {
    let content = "# Just some markdown\n\nNo frontmatter here.";
    let (fm, instructions) = parse_frontmatter(content).unwrap();
    assert!(fm.is_mapping());
    assert!(instructions.contains("Just some markdown"));
}

#[test]
fn test_binary_exists() {
    let manager = SkillManager::new(std::env::temp_dir());
    // 'ls' or 'dir' should exist on most systems
    #[cfg(unix)]
    assert!(manager.binary_exists("ls"));
    #[cfg(windows)]
    assert!(manager.binary_exists("cmd"));
}

#[test]
fn test_gate_check_always() {
    let manager = SkillManager::new(std::env::temp_dir());
    let skill = Skill {
        name: "test".into(),
        description: None,
        path: PathBuf::new(),
        enabled: true,
        instructions: String::new(),
        metadata: SkillMetadata {
            always: true,
            ..Default::default()
        },
        source: SkillSource::Local,
        linked_secrets: vec![],
    };
    let result = manager.check_gates(&skill);
    assert!(result.passed);
}

#[test]
fn test_gate_check_missing_bin() {
    let manager = SkillManager::new(std::env::temp_dir());
    let skill = Skill {
        name: "test".into(),
        description: None,
        path: PathBuf::new(),
        enabled: true,
        instructions: String::new(),
        metadata: SkillMetadata {
            requires: SkillRequirements {
                bins: vec!["nonexistent_binary_12345".into()],
                ..Default::default()
            },
            ..Default::default()
        },
        source: SkillSource::Local,
        linked_secrets: vec![],
    };
    let result = manager.check_gates(&skill);
    assert!(!result.passed);
    assert!(
        result
            .missing_bins
            .contains(&"nonexistent_binary_12345".to_string())
    );
}

#[test]
fn test_generate_prompt_context() {
    let mut manager = SkillManager::new(std::env::temp_dir());
    manager.skills.push(Skill {
        name: "test-skill".into(),
        description: Some("Does testing".into()),
        path: PathBuf::from("/skills/test/SKILL.md"),
        enabled: true,
        instructions: "Test instructions".into(),
        metadata: SkillMetadata::default(),
        source: SkillSource::Local,
        linked_secrets: vec![],
    });
    let context = manager.generate_prompt_context();
    assert!(context.contains("test-skill"));
    assert!(context.contains("Does testing"));
    assert!(context.contains("<available_skills>"));
}

#[test]
fn test_link_and_unlink_secret() {
    let mut manager = SkillManager::new(std::env::temp_dir());
    manager.skills.push(Skill {
        name: "deploy".into(),
        description: Some("Deploy things".into()),
        path: PathBuf::from("/skills/deploy/SKILL.md"),
        enabled: true,
        instructions: String::new(),
        metadata: SkillMetadata::default(),
        source: SkillSource::Local,
        linked_secrets: vec![],
    });

    manager.link_secret("deploy", "AWS_KEY").unwrap();
    manager.link_secret("deploy", "AWS_SECRET").unwrap();
    assert_eq!(
        manager.get_linked_secrets("deploy"),
        vec!["AWS_KEY", "AWS_SECRET"]
    );

    // Linking the same secret again should not duplicate.
    manager.link_secret("deploy", "AWS_KEY").unwrap();
    assert_eq!(manager.get_linked_secrets("deploy").len(), 2);

    manager.unlink_secret("deploy", "AWS_KEY").unwrap();
    assert_eq!(manager.get_linked_secrets("deploy"), vec!["AWS_SECRET"]);
}

#[test]
fn test_link_secret_skill_not_found() {
    let mut manager = SkillManager::new(std::env::temp_dir());
    assert!(manager.link_secret("nonexistent", "key").is_err());
}

#[test]
fn test_skill_info() {
    let mut manager = SkillManager::new(std::env::temp_dir());
    manager.skills.push(Skill {
        name: "web-scrape".into(),
        description: Some("Scrape web pages".into()),
        path: PathBuf::from("/skills/web-scrape/SKILL.md"),
        enabled: true,
        instructions: String::new(),
        metadata: SkillMetadata::default(),
        source: SkillSource::Registry {
            registry_url: "https://registry.clawhub.dev/api/v1".into(),
            version: "1.0.0".into(),
        },
        linked_secrets: vec!["SCRAPER_KEY".into()],
    });

    let info = manager.skill_info("web-scrape").unwrap();
    assert!(info.contains("web-scrape"));
    assert!(info.contains("registry"));
    assert!(info.contains("SCRAPER_KEY"));
    assert!(manager.skill_info("nonexistent").is_none());
}

#[test]
fn test_remove_skill() {
    let mut manager = SkillManager::new(std::env::temp_dir());
    manager.skills.push(Skill {
        name: "temp-skill".into(),
        description: None,
        path: PathBuf::from("/nonexistent/SKILL.md"),
        enabled: true,
        instructions: String::new(),
        metadata: SkillMetadata::default(),
        source: SkillSource::Local,
        linked_secrets: vec![],
    });
    assert_eq!(manager.get_skills().len(), 1);
    manager.remove_skill("temp-skill").unwrap();
    assert_eq!(manager.get_skills().len(), 0);
    assert!(manager.remove_skill("temp-skill").is_err());
}

#[test]
fn test_skill_source_default() {
    assert_eq!(SkillSource::default(), SkillSource::Local);
}

#[test]
fn test_base64_decode() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let encoded = "SGVsbG8=";
    let decoded = STANDARD.decode(encoded).unwrap();
    assert_eq!(decoded, b"Hello");
}

/// `create_skill` builds a directory path from a caller-supplied name, so it
/// is the third door into the same `join` that `contained_path` guards for
/// archive entries and slugs. It had neither check until #428/#430.
#[cfg(test)]
mod create_skill_name_tests {
    use super::super::{SkillError, SkillManager};

    fn refuses(name: &str) -> String {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut manager = SkillManager::new(dir.path().to_path_buf());
        match manager.create_skill(name, "d", "i", None) {
            Err(SkillError::InvalidName { reason, .. }) => reason.to_string(),
            Err(other) => panic!("expected InvalidName for {name:?}, got {other:?}"),
            Ok(path) => panic!("{name:?} was accepted and wrote to {}", path.display()),
        }
    }

    /// `..` has no slash, no space, and is not empty — it passed every check
    /// the function had, and `skills_dir.join("..")` wrote SKILL.md a
    /// directory above the skills directory.
    #[test]
    fn a_name_that_climbs_out_is_refused() {
        assert!(refuses("..").contains("dot"));
        assert!(refuses(".hidden").contains("dot"));
    }

    /// On Windows `join` with `C:evil` discards the base entirely and resolves
    /// against drive C's current directory. On Unix `Path` reads it as one
    /// ordinary filename, so a check built on `Path` alone would pass here and
    /// leave Windows open — the same reason `contained_path` checks the string.
    #[test]
    fn a_drive_relative_name_is_refused_on_every_platform() {
        assert!(refuses("C:evil").contains("drive"));
        assert!(refuses("Z:").contains("drive"));
    }

    /// And an ordinary name still works — a rule that refused everything would
    /// satisfy both tests above.
    #[test]
    fn an_ordinary_name_is_accepted() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut manager = SkillManager::new(dir.path().to_path_buf());
        let path = manager
            .create_skill("my-skill", "d", "i", None)
            .expect("an ordinary name is fine");
        assert!(
            path.starts_with(dir.path()),
            "wrote outside: {}",
            path.display()
        );
    }
}
