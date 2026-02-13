//! Golden file tests for CLI help output.
//!
//! These tests compare current help output against stored golden files.
//! To update golden files, run: `UPDATE_GOLDEN=1 cargo test`

use std::fs;
use std::path::Path;
use std::process::Command;

const GOLDEN_DIR: &str = "tests/golden";

/// Get help output for a command
fn get_help(args: &[&str]) -> String {
    let mut cmd_args = vec!["run", "--quiet", "--"];
    cmd_args.extend(args);
    cmd_args.push("--help");

    let output = Command::new("cargo")
        .args(&cmd_args)
        .output()
        .expect("Failed to execute rustyclaw");

    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Compare output against golden file, updating if UPDATE_GOLDEN=1
fn check_golden(name: &str, actual: &str) {
    let golden_path = Path::new(GOLDEN_DIR).join(format!("{}.txt", name));
    
    // If UPDATE_GOLDEN is set, write the new golden file
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        fs::create_dir_all(GOLDEN_DIR).ok();
        fs::write(&golden_path, actual).expect("Failed to write golden file");
        println!("Updated golden file: {}", golden_path.display());
        return;
    }

    // Otherwise, compare against existing golden file
    if !golden_path.exists() {
        panic!(
            "Golden file not found: {}\n\
             Run with UPDATE_GOLDEN=1 to create it.\n\
             Actual output:\n{}",
            golden_path.display(),
            actual
        );
    }

    let expected = fs::read_to_string(&golden_path).expect("Failed to read golden file");
    
    if actual != expected {
        // Show diff
        let actual_lines: Vec<&str> = actual.lines().collect();
        let expected_lines: Vec<&str> = expected.lines().collect();
        
        let mut diff = String::new();
        diff.push_str(&format!("Golden file mismatch: {}\n", golden_path.display()));
        diff.push_str("Run with UPDATE_GOLDEN=1 to update.\n\n");
        
        for (i, (a, e)) in actual_lines.iter().zip(expected_lines.iter()).enumerate() {
            if a != e {
                diff.push_str(&format!("Line {}: \n  expected: {}\n  actual:   {}\n", i + 1, e, a));
            }
        }
        
        if actual_lines.len() != expected_lines.len() {
            diff.push_str(&format!(
                "\nLine count mismatch: expected {}, got {}\n",
                expected_lines.len(),
                actual_lines.len()
            ));
        }
        
        panic!("{}", diff);
    }
}

// ── Main Help ───────────────────────────────────────────────────────────────

#[test]
fn test_golden_main_help() {
    let output = get_help(&[]);
    check_golden("help_main", &output);
}

// ── Subcommand Help ─────────────────────────────────────────────────────────

#[test]
fn test_golden_setup_help() {
    let output = get_help(&["setup"]);
    check_golden("help_setup", &output);
}

#[test]
fn test_golden_gateway_help() {
    let output = get_help(&["gateway"]);
    check_golden("help_gateway", &output);
}

#[test]
fn test_golden_skills_help() {
    let output = get_help(&["skills"]);
    check_golden("help_skills", &output);
}

#[test]
fn test_golden_doctor_help() {
    let output = get_help(&["doctor"]);
    check_golden("help_doctor", &output);
}

#[test]
fn test_golden_command_help() {
    let output = get_help(&["command"]);
    check_golden("help_command", &output);
}

#[test]
fn test_golden_status_help() {
    let output = get_help(&["status"]);
    check_golden("help_status", &output);
}

#[test]
fn test_golden_tui_help() {
    let output = get_help(&["tui"]);
    check_golden("help_tui", &output);
}

#[test]
fn test_golden_configure_help() {
    let output = get_help(&["configure"]);
    check_golden("help_configure", &output);
}

// ── Gateway Subcommands ─────────────────────────────────────────────────────

#[test]
fn test_golden_gateway_start_help() {
    let output = get_help(&["gateway", "start"]);
    check_golden("help_gateway_start", &output);
}

#[test]
fn test_golden_gateway_stop_help() {
    let output = get_help(&["gateway", "stop"]);
    check_golden("help_gateway_stop", &output);
}

#[test]
fn test_golden_gateway_status_help() {
    let output = get_help(&["gateway", "status"]);
    check_golden("help_gateway_status", &output);
}

// ── Skills Subcommands ──────────────────────────────────────────────────────

#[test]
fn test_golden_skills_list_help() {
    let output = get_help(&["skills", "list"]);
    check_golden("help_skills_list", &output);
}

// ── Version Output ──────────────────────────────────────────────────────────

#[test]
fn test_golden_version() {
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--", "--version"])
        .output()
        .expect("Failed to execute rustyclaw");

    let version = String::from_utf8_lossy(&output.stdout).to_string();
    
    // Version changes frequently, so just verify format
    assert!(
        version.contains("rustyclaw") || version.contains("RustyClaw"),
        "Version should contain app name"
    );
    assert!(
        version.contains('.'),
        "Version should contain version number"
    );
}
