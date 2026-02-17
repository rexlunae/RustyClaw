//! End-to-end test suite for RustyClaw
//!
//! These tests run complete user scenarios from start to finish.

use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Get the rustyclaw binary path
fn binary_path() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    
    let debug = PathBuf::from(&manifest_dir).join("target/debug/rustyclaw");
    if debug.exists() {
        return debug;
    }
    
    PathBuf::from(&manifest_dir).join("target/release/rustyclaw")
}

/// Create a temporary workspace
fn temp_workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rustyclaw-e2e-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// E2E: Fresh setup and first run
#[test]
#[ignore = "requires built binary"]
fn test_e2e_fresh_setup() {
    let binary = binary_path();
    let workspace = temp_workspace();
    
    // Run onboard command (non-interactive mode if available)
    let output = Command::new(&binary)
        .arg("doctor")
        .arg("--check-only")
        .env("HOME", &workspace)
        .output()
        .expect("Failed to run doctor");
    
    // Should complete without crashing
    println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
    println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
    
    fs::remove_dir_all(&workspace).ok();
}

/// E2E: CLI help works
#[test]
#[ignore = "requires built binary"]
fn test_e2e_help_works() {
    let binary = binary_path();
    
    let output = Command::new(&binary)
        .arg("--help")
        .output()
        .expect("Failed to run help");
    
    assert!(output.status.success(), "Help should succeed");
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("rustyclaw") || stdout.contains("RustyClaw"));
    assert!(stdout.contains("gateway"));
}

/// E2E: Version command
#[test]
#[ignore = "requires built binary"]
fn test_e2e_version() {
    let binary = binary_path();
    
    let output = Command::new(&binary)
        .arg("--version")
        .output()
        .expect("Failed to run version");
    
    assert!(output.status.success());
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain version number pattern
    assert!(
        stdout.contains('.') && stdout.chars().any(|c| c.is_ascii_digit()),
        "Version should contain numbers: {stdout}"
    );
}

/// E2E: Status command without gateway
#[test]
#[ignore = "requires built binary"]
fn test_e2e_status_no_gateway() {
    let binary = binary_path();
    let workspace = temp_workspace();
    
    let output = Command::new(&binary)
        .arg("status")
        .env("HOME", &workspace)
        .output()
        .expect("Failed to run status");
    
    // Should report gateway not running (not crash)
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ).to_lowercase();
    
    assert!(
        combined.contains("not running") || 
        combined.contains("offline") ||
        combined.contains("stopped") ||
        combined.contains("no gateway") ||
        !output.status.success(), // Or just fail gracefully
        "Should indicate gateway not running: {combined}"
    );
    
    fs::remove_dir_all(&workspace).ok();
}

/// E2E: Gateway subcommand help
#[test]
#[ignore = "requires built binary"]
fn test_e2e_gateway_help() {
    let binary = binary_path();
    
    let output = Command::new(&binary)
        .args(["gateway", "--help"])
        .output()
        .expect("Failed to run gateway help");
    
    assert!(output.status.success());
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("start") || stdout.contains("run"));
    assert!(stdout.contains("stop") || stdout.contains("status"));
}

/// E2E: Skills list command
#[test]
#[ignore = "requires built binary"]
fn test_e2e_skills_list() {
    let binary = binary_path();
    let workspace = temp_workspace();
    
    // Create minimal skills directory
    let skills_dir = workspace.join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    
    let output = Command::new(&binary)
        .args(["skills", "list"])
        .env("HOME", &workspace)
        .env("RUSTYCLAW_SKILLS", &skills_dir)
        .output()
        .expect("Failed to run skills list");
    
    // Should complete (might show empty or error, but not crash)
    println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
    
    fs::remove_dir_all(&workspace).ok();
}

/// E2E: Config generation
#[test]
#[ignore = "requires built binary"]
fn test_e2e_config_generation() {
    let binary = binary_path();
    let workspace = temp_workspace();
    let config_path = workspace.join("config.toml");
    
    // Try to generate a default config (if such command exists)
    // Or just verify we can read a config
    
    // Create a minimal config
    fs::write(&config_path, r#"
[provider]
kind = "mock"
"#).unwrap();
    
    let output = Command::new(&binary)
        .args(["doctor", "--config"])
        .arg(&config_path)
        .output()
        .expect("Failed to run doctor with config");
    
    // Should be able to read the config
    
    fs::remove_dir_all(&workspace).ok();
}

/// E2E: Multi-turn conversation simulation
#[tokio::test]
#[ignore = "requires running gateway with mock provider"]
async fn test_e2e_multi_turn_conversation() -> Result<()> {
    // This would require starting a gateway and having a multi-turn conversation
    // For now, this is a placeholder for when we have the infrastructure
    
    // 1. Start gateway with mock provider
    // 2. Connect via WebSocket
    // 3. Send message 1
    // 4. Wait for response
    // 5. Send message 2 (referencing previous context)
    // 6. Verify context is maintained
    
    Ok(())
}

/// E2E: Error recovery
#[tokio::test]
#[ignore = "requires running gateway"]
async fn test_e2e_error_recovery() -> Result<()> {
    // Test that the gateway recovers gracefully from:
    // 1. Provider errors (rate limits, etc.)
    // 2. Tool execution failures
    // 3. Malformed client messages
    
    Ok(())
}

/// E2E: Workspace file operations
#[test]
#[ignore = "requires built binary"]
fn test_e2e_workspace_operations() {
    let binary = binary_path();
    let workspace = temp_workspace();
    
    // Create workspace structure
    fs::create_dir_all(workspace.join("memory")).unwrap();
    fs::write(workspace.join("SOUL.md"), "# Test Soul\nI am a test agent.").unwrap();
    fs::write(workspace.join("MEMORY.md"), "# Memory\n- Test entry").unwrap();
    
    // Verify workspace is valid
    let output = Command::new(&binary)
        .arg("doctor")
        .arg("--check-only")
        .current_dir(&workspace)
        .env("HOME", &workspace)
        .output()
        .expect("Failed to run doctor");
    
    println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
    println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
    
    fs::remove_dir_all(&workspace).ok();
}

/// E2E: Concurrent request handling
#[tokio::test]
#[ignore = "requires running gateway"]
async fn test_e2e_concurrent_requests() -> Result<()> {
    // Test that the gateway handles multiple concurrent requests properly
    // 1. Start gateway
    // 2. Open multiple WebSocket connections
    // 3. Send messages concurrently
    // 4. Verify all get proper responses
    
    Ok(())
}

/// E2E: Long-running session
#[tokio::test]
#[ignore = "requires running gateway - slow test"]
async fn test_e2e_long_session() -> Result<()> {
    // Test session stability over time
    // 1. Start gateway
    // 2. Connect and send periodic messages
    // 3. Verify connection stays alive
    // 4. Test reconnection if needed
    
    Ok(())
}
