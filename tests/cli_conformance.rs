//! CLI conformance tests - golden file testing for help output and behavior.
//!
//! These tests verify that RustyClaw's CLI matches expected behavior and that
//! help text remains stable across versions.

use std::process::Command;

/// Helper to run rustyclaw with args and capture output
fn run_rustyclaw(args: &[&str]) -> (String, String, i32) {
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--"])
        .args(args)
        .output()
        .expect("Failed to execute rustyclaw");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let code = output.status.code().unwrap_or(-1);

    (stdout, stderr, code)
}

// ── Help Output Tests ───────────────────────────────────────────────────────

#[test]
fn test_help_shows_usage() {
    let (stdout, _, code) = run_rustyclaw(&["--help"]);
    
    assert_eq!(code, 0, "Help should exit with code 0");
    assert!(stdout.contains("RustyClaw"), "Should contain app name");
    assert!(stdout.contains("Usage:"), "Should contain usage section");
}

#[test]
fn test_help_shows_subcommands() {
    let (stdout, _, _) = run_rustyclaw(&["--help"]);
    
    // All expected subcommands
    let expected_commands = [
        "setup",
        "configure",
        "doctor",
        "tui",
        "command",
        "status",
        "gateway",
        "skills",
    ];
    
    for cmd in expected_commands {
        assert!(
            stdout.to_lowercase().contains(cmd),
            "Help should list '{}' subcommand",
            cmd
        );
    }
}

#[test]
fn test_help_shows_global_options() {
    let (stdout, _, _) = run_rustyclaw(&["--help"]);
    
    // Global options matching OpenClaw
    assert!(stdout.contains("--config") || stdout.contains("-c"), "Should have --config/-c");
    assert!(stdout.contains("--profile"), "Should have --profile");
    assert!(stdout.contains("--no-color"), "Should have --no-color");
}

#[test]
fn test_version_output() {
    let (stdout, _, code) = run_rustyclaw(&["--version"]);
    
    assert_eq!(code, 0, "Version should exit with code 0");
    assert!(stdout.contains("rustyclaw") || stdout.contains("RustyClaw"), "Should contain app name");
    // Version format: rustyclaw X.Y.Z
    assert!(stdout.contains('.'), "Should contain version number with dots");
}

// ── Subcommand Help Tests ───────────────────────────────────────────────────

#[test]
fn test_setup_help() {
    let (stdout, _, code) = run_rustyclaw(&["setup", "--help"]);
    
    assert_eq!(code, 0);
    assert!(stdout.contains("workspace") || stdout.contains("wizard"), "Setup should mention workspace or wizard");
}

#[test]
fn test_gateway_help() {
    let (stdout, _, code) = run_rustyclaw(&["gateway", "--help"]);
    
    assert_eq!(code, 0);
    // Gateway subcommands
    let expected = ["start", "stop", "status"];
    for cmd in expected {
        assert!(
            stdout.to_lowercase().contains(cmd),
            "Gateway help should list '{}' subcommand",
            cmd
        );
    }
}

#[test]
fn test_skills_help() {
    let (stdout, _, code) = run_rustyclaw(&["skills", "--help"]);
    
    assert_eq!(code, 0);
    assert!(stdout.to_lowercase().contains("list") || stdout.to_lowercase().contains("skill"),
            "Skills help should mention list or skill management");
}

#[test]
fn test_doctor_help() {
    let (stdout, _, code) = run_rustyclaw(&["doctor", "--help"]);
    
    assert_eq!(code, 0);
    assert!(stdout.contains("repair") || stdout.contains("check") || stdout.contains("health"),
            "Doctor should mention repair/check/health");
}

#[test]
fn test_command_help() {
    let (stdout, _, code) = run_rustyclaw(&["command", "--help"]);
    
    assert_eq!(code, 0);
    assert!(stdout.contains("message") || stdout.contains("send") || stdout.contains("command"),
            "Command should describe sending messages");
}

// ── Exit Code Tests ─────────────────────────────────────────────────────────

#[test]
fn test_unknown_command_exits_nonzero() {
    let (_, stderr, code) = run_rustyclaw(&["nonexistent-command-12345"]);
    
    assert_ne!(code, 0, "Unknown command should exit with non-zero code");
    assert!(
        stderr.contains("error") || stderr.contains("unrecognized") || stderr.contains("invalid"),
        "Should show error for unknown command"
    );
}

#[test]
fn test_invalid_flag_exits_nonzero() {
    let (_, stderr, code) = run_rustyclaw(&["--nonexistent-flag-12345"]);
    
    assert_ne!(code, 0, "Invalid flag should exit with non-zero code");
    assert!(
        stderr.contains("error") || stderr.contains("unexpected") || stderr.contains("invalid"),
        "Should show error for invalid flag"
    );
}

// ── Environment Variable Tests ──────────────────────────────────────────────

#[test]
fn test_env_var_config_recognized() {
    // This test verifies the env var is documented in help
    let (stdout, _, _) = run_rustyclaw(&["--help"]);
    
    // The help should mention RUSTYCLAW_CONFIG or similar
    // (clap shows env vars in help when configured)
    assert!(
        stdout.contains("RUSTYCLAW") || stdout.contains("env"),
        "Help should mention environment variables"
    );
}

// ── Status Command Tests ────────────────────────────────────────────────────

#[test]
fn test_status_runs_without_gateway() {
    let (stdout, stderr, code) = run_rustyclaw(&["status"]);
    
    // Status should work even without a running gateway
    // It might show "not connected" or similar, but shouldn't crash
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        code == 0 || combined.contains("not") || combined.contains("error") || combined.contains("offline"),
        "Status should either succeed or show meaningful error"
    );
}

// ── Config Subcommand Tests ─────────────────────────────────────────────────

#[test]
fn test_config_get_help() {
    let (stdout, _, code) = run_rustyclaw(&["config", "--help"]);
    
    // Config should have get/set subcommands
    assert!(code == 0 || code == 2, "Config help should work");
    let combined = stdout.to_lowercase();
    assert!(
        combined.contains("get") || combined.contains("set") || combined.contains("config"),
        "Config should mention get/set operations"
    );
}
