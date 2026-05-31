//! (onboard submodule)

use std::io::{self, BufRead, Write};

use anyhow::Result;

use crate::prompts::arrow_select;
use rustyclaw_core::config::Config;
use rustyclaw_core::theme as t;

/// Recommended skills with descriptions (excludes messaging — handled separately).
struct RecommendedSkill {
    slug: &'static str,
    display: &'static str,
    description: &'static str,
    category: &'static str,
}

const RECOMMENDED_SKILLS: &[RecommendedSkill] = &[
    // Development
    RecommendedSkill {
        slug: "github",
        display: "GitHub",
        description: "GitHub operations via gh CLI — issues, PRs, CI, code review",
        category: "Development",
    },
    // Productivity
    RecommendedSkill {
        slug: "weather",
        display: "Weather",
        description: "Current weather and forecasts via wttr.in or Open-Meteo",
        category: "Productivity",
    },
    RecommendedSkill {
        slug: "bluesky",
        display: "Bluesky",
        description: "Bluesky social network — post, read timeline, follow, like, repost",
        category: "Social",
    },
];

/// Walk the user through installing recommended skills from ClawHub.
pub(crate) fn setup_recommended_skills(_reader: &mut impl BufRead, config: &Config) -> Result<()> {
    println!("{}", t::heading("Install recommended skills (optional):"));
    println!();
    println!("  Skills extend RustyClaw with new capabilities. These are");
    println!("  installed from ClawHub and don't require recompilation.");
    println!();

    // Check if clawhub CLI is available
    let clawhub_available = std::process::Command::new("clawhub")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !clawhub_available {
        println!("  {}", t::warn("clawhub CLI not found."));
        println!(
            "  Install it with: {}",
            t::accent_bright("npm install -g clawhub")
        );
        println!(
            "  Then run {} to add skills.",
            t::accent_bright("rustyclaw skills install <name>")
        );
        println!();
        return Ok(());
    }

    let skills_dir = config.workspace_dir().join("skills");

    // Build list of skills not yet installed
    let mut available: Vec<usize> = Vec::new();
    for (i, skill) in RECOMMENDED_SKILLS.iter().enumerate() {
        let skill_path = skills_dir.join(skill.slug);
        if !skill_path.exists() {
            available.push(i);
        }
    }

    if available.is_empty() {
        println!(
            "  {}",
            t::icon_ok("All recommended skills are already installed.")
        );
        println!();
        return Ok(());
    }

    // Group by category for display
    let mut by_category: std::collections::BTreeMap<&str, Vec<usize>> =
        std::collections::BTreeMap::new();
    for &i in &available {
        by_category
            .entry(RECOMMENDED_SKILLS[i].category)
            .or_default()
            .push(i);
    }

    // Display available skills
    println!("  {}", t::bold("Available skills:"));
    println!();
    for (category, indices) in &by_category {
        println!("  {}", t::accent(category));
        for &i in indices {
            let skill = &RECOMMENDED_SKILLS[i];
            println!(
                "    {} — {}",
                t::accent_bright(skill.display),
                t::muted(skill.description)
            );
        }
        println!();
    }

    // Let user select which to install
    let mut choices: Vec<String> = available
        .iter()
        .map(|&i| RECOMMENDED_SKILLS[i].display.to_string())
        .collect();
    choices.push("Install all recommended".to_string());
    choices.push("Skip — install none".to_string());

    let selected = match arrow_select(&choices, "Select skills to install:")? {
        None => return Ok(()),
        Some(idx) => idx,
    };

    println!();

    let to_install: Vec<usize> = if selected == choices.len() - 1 {
        // Skip
        println!(
            "  {}",
            t::muted("Skipping skill installation. You can install them later with:")
        );
        println!(
            "    {}",
            t::accent_bright("rustyclaw skills install <name>")
        );
        println!();
        return Ok(());
    } else if selected == choices.len() - 2 {
        // Install all
        available.clone()
    } else {
        // Single skill
        vec![available[selected]]
    };

    // Install selected skills
    for &i in &to_install {
        let skill = &RECOMMENDED_SKILLS[i];
        print!("  {} Installing {}...", t::muted("⠋"), skill.display);
        io::stdout().flush()?;

        let output = std::process::Command::new("clawhub")
            .args([
                "install",
                skill.slug,
                "--dir",
                &skills_dir.to_string_lossy(),
            ])
            .output();

        // Clear spinner
        print!("\r{}\r", " ".repeat(60));

        match output {
            Ok(out) if out.status.success() => {
                println!("  {}", t::icon_ok(&format!("Installed {}", skill.display)));
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                println!(
                    "  {}",
                    t::icon_warn(&format!(
                        "Failed to install {}: {}",
                        skill.display,
                        stderr.trim()
                    ))
                );
            }
            Err(e) => {
                println!(
                    "  {}",
                    t::icon_warn(&format!("Failed to install {}: {}", skill.display, e))
                );
            }
        }
    }

    println!();
    Ok(())
}
