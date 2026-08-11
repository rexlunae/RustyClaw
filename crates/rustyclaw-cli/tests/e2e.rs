//! End-to-end test suite for RustyClaw
//!
//! These tests run complete user scenarios from start to finish.

mod common;

// The isolation this file already had, now shared. `exit_codes.rs` ran
// `gateway stop` without it and killed the developer's running gateway on
// every `cargo test --workspace`.
use common::scratch_command;

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The binary under test.
///
/// Cargo builds it as a dependency of this test and passes the path in, so
/// the tests cannot silently run against a stale build — or none at all,
/// which is what the old profile-directory guess did.
fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyclaw"))
}

/// A workspace of this test's own.
///
/// Named per test, not just per process. Sharing one directory across the
/// file made these mutually destructive: every test ends by removing it, so
/// one finishing while another was mid-run deleted the directory out from
/// under it — `current_dir` on a path that no longer existed, surfacing as a
/// NotFound from the spawn rather than anything about the test. Harmless
/// while nothing ran them; a flake the moment they did.
fn temp_workspace(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rustyclaw-e2e-{}-{test}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The checks `doctor` promises to run, as they appear in its report.
///
/// Named here so a check that is quietly dropped fails a test rather than
/// going unnoticed — the report is the whole product of the command.
const DOCTOR_CHECKS: &[&str] = &[
    "Config file",
    "Workspace dir",
    "Credentials dir",
    "SOUL.md",
    "Skills dir",
];

/// Run `doctor` against a given HOME and return its combined output.
///
/// `--non-interactive` rather than the `--check-only` these tests used to
/// pass: that flag does not exist, so clap rejected the invocation with a
/// usage error before `doctor` ran. Neither test asserted anything, so both
/// passed on the usage error — advertised as end-to-end coverage of the
/// health check while never reaching it.
fn run_doctor(binary: &Path, home: &Path) -> String {
    let output = scratch_command(binary, home)
        .arg("doctor")
        .arg("--non-interactive")
        .output()
        .expect("Failed to run doctor");
    assert!(
        output.status.success(),
        "doctor exited with {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// E2E: Fresh setup and first run
#[test]
fn test_e2e_fresh_setup() {
    let binary = binary_path();
    let workspace = temp_workspace("fresh_setup");

    let report = run_doctor(&binary, &workspace);

    for check in DOCTOR_CHECKS {
        assert!(
            report.contains(check),
            "doctor did not report the {check:?} check\n{report}"
        );
    }
    // Nothing has been set up, so it must say so rather than passing a
    // workspace it never looked at.
    assert!(
        report.contains("Some checks failed"),
        "a bare HOME should not come up clean\n{report}"
    );

    fs::remove_dir_all(&workspace).ok();
}

/// E2E: CLI help works
#[test]
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
fn test_e2e_status_no_gateway() {
    let binary = binary_path();
    let workspace = temp_workspace("status_no_gateway");

    let output = scratch_command(&binary, &workspace)
        .arg("status")
        .output()
        .expect("Failed to run status");

    // Should report gateway not running (not crash)
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_lowercase();

    assert!(
        combined.contains("not running")
            || combined.contains("offline")
            || combined.contains("stopped")
            || combined.contains("no gateway")
            || !output.status.success(), // Or just fail gracefully
        "Should indicate gateway not running: {combined}"
    );

    fs::remove_dir_all(&workspace).ok();
}

/// E2E: Gateway subcommand help
#[test]
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
fn test_e2e_skills_list() {
    let binary = binary_path();
    let workspace = temp_workspace("skills_list");

    // Create minimal skills directory
    let skills_dir = workspace.join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // This one wants `RUSTYCLAW_SKILLS` — set after the scrub, so it is the
    // test's value rather than whatever the developer exported.
    let output = scratch_command(&binary, &workspace)
        .args(["skills", "list"])
        .env("RUSTYCLAW_SKILLS", &skills_dir)
        .output()
        .expect("Failed to run skills list");

    // Should complete (might show empty or error, but not crash)
    println!("stdout: {}", String::from_utf8_lossy(&output.stdout));

    fs::remove_dir_all(&workspace).ok();
}

/// E2E: Config generation
#[test]
fn test_e2e_config_generation() {
    let binary = binary_path();
    let workspace = temp_workspace("config_generation");
    let config_path = workspace.join("config.toml");

    // A config that parses as TOML but is not a valid `Config`. This used to
    // be written, passed to `doctor`, and the result discarded into `let _`
    // with nothing asserted — so the test passed whatever happened, which is
    // the pattern this PR exists to remove.
    fs::write(
        &config_path,
        r#"
[provider]
kind = "mock"
"#,
    )
    .unwrap();

    let output = scratch_command(&binary, &workspace)
        .args(["doctor", "--non-interactive", "--config"])
        .arg(&config_path)
        .output()
        .expect("Failed to run doctor with config");
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        report.contains("Failed to parse config"),
        "an unreadable config should be reported, not passed over\n{report}"
    );
    assert!(
        report.contains("✗ Config file"),
        "the config check should not pass on a config that failed to parse\n{report}"
    );
    // Quarantine is deliberate and the message says how to undo it, so the
    // file must actually be moved rather than left to fail again next run.
    assert!(
        !config_path.exists(),
        "the unreadable config should have been moved aside"
    );
    let quarantined: Vec<_> = fs::read_dir(&workspace)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
        .collect();
    assert_eq!(
        quarantined.len(),
        1,
        "exactly one quarantined copy should be left behind"
    );

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
fn test_e2e_workspace_operations() {
    let binary = binary_path();
    let workspace = temp_workspace("workspace_operations");

    // Before: nothing on disk, so the workspace check should fail.
    let before = run_doctor(&binary, &workspace);
    assert!(
        before.contains("✗ Workspace dir"),
        "an empty HOME should fail the workspace check\n{before}"
    );

    // Create the directory doctor looks for, and nothing else.
    fs::create_dir_all(workspace.join(".rustyclaw").join("workspace")).unwrap();

    // After: the same check should pass, and only that one should move.
    let after = run_doctor(&binary, &workspace);
    assert!(
        after.contains("✓ Workspace dir"),
        "doctor did not notice the workspace directory\n{after}"
    );
    assert!(
        after.contains("✗ Config file"),
        "creating a workspace should not make unrelated checks pass\n{after}"
    );

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
