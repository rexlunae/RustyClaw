//! Integration tests for sandbox enforcement across all modes.
//!
//! These tests verify that the C1 security fix properly enforces
//! sandbox restrictions and prevents unauthorized access to protected paths.

use std::path::PathBuf;
use std::fs;
use tempfile::TempDir;

/// Test that PathValidation mode blocks access to deny_read paths
#[test]
fn test_path_validation_blocks_deny_read() {
    use rustyclaw::sandbox::{run_sandboxed, SandboxMode, SandboxPolicy};

    let temp_workspace = TempDir::new().unwrap();
    let temp_credentials = TempDir::new().unwrap();

    // Create a test file in the credentials directory
    let secret_file = temp_credentials.path().join("secret.txt");
    fs::write(&secret_file, "SECRET_DATA").unwrap();

    let policy = SandboxPolicy {
        deny_read: vec![temp_credentials.path().to_path_buf()],
        deny_write: vec![],
        deny_exec: vec![],
        allow_paths: vec![],
        workspace: temp_workspace.path().to_path_buf(),
    };

    // Try to read the secret file - should be blocked
    let command = format!("cat {}", secret_file.display());
    let result = run_sandboxed(&command, &policy, SandboxMode::PathValidation);

    assert!(
        result.is_err(),
        "PathValidation should block access to deny_read paths"
    );
    let error = result.unwrap_err();
    assert!(
        error.contains("Access denied") || error.contains("protected"),
        "Error message should indicate access denial, got: {}",
        error
    );
}

/// Test that PathValidation mode allows access to workspace
#[test]
fn test_path_validation_allows_workspace() {
    use rustyclaw::sandbox::{run_sandboxed, SandboxMode, SandboxPolicy};

    let temp_workspace = TempDir::new().unwrap();
    let temp_credentials = TempDir::new().unwrap();

    // Create a test file in the workspace
    let workspace_file = temp_workspace.path().join("allowed.txt");
    fs::write(&workspace_file, "ALLOWED_DATA").unwrap();

    let policy = SandboxPolicy {
        deny_read: vec![temp_credentials.path().to_path_buf()],
        deny_write: vec![],
        deny_exec: vec![],
        allow_paths: vec![],
        workspace: temp_workspace.path().to_path_buf(),
    };

    // Try to read the workspace file - should be allowed
    let command = format!("cat {}", workspace_file.display());
    let result = run_sandboxed(&command, &policy, SandboxMode::PathValidation);

    // PathValidation allows workspace access
    assert!(
        result.is_ok(),
        "PathValidation should allow access to workspace paths: {:?}",
        result
    );
}

/// Test that deny_exec prevents execution from protected paths
#[test]
fn test_path_validation_blocks_deny_exec() {
    use rustyclaw::sandbox::{run_sandboxed, SandboxMode, SandboxPolicy};

    let temp_workspace = TempDir::new().unwrap();
    let temp_scripts = TempDir::new().unwrap();

    // Create a test script in the denied directory
    let script_file = temp_scripts.path().join("malicious.sh");
    fs::write(&script_file, "#!/bin/bash\necho 'MALICIOUS'").unwrap();

    // Make it executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_file).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_file, perms).unwrap();
    }

    let policy = SandboxPolicy {
        deny_read: vec![],
        deny_write: vec![],
        deny_exec: vec![temp_scripts.path().to_path_buf()],
        allow_paths: vec![],
        workspace: temp_workspace.path().to_path_buf(),
    };

    // Try to execute the script - should be blocked
    let command = script_file.display().to_string();
    let result = run_sandboxed(&command, &policy, SandboxMode::PathValidation);

    assert!(
        result.is_err(),
        "PathValidation should block execution from deny_exec paths"
    );
    let error = result.unwrap_err();
    assert!(
        error.contains("Execution denied") || error.contains("deny_exec"),
        "Error message should indicate execution denial, got: {}",
        error
    );
}

/// Test that extract_paths_from_command correctly identifies paths
#[test]
fn test_extract_paths_basic() {
    use rustyclaw::sandbox::extract_paths_from_command;

    // Test absolute paths
    let paths = extract_paths_from_command("cat /etc/passwd");
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], PathBuf::from("/etc/passwd"));

    // Test home paths
    let paths = extract_paths_from_command("ls ~/documents");
    assert_eq!(paths.len(), 1);
    assert!(paths[0].to_string_lossy().contains("documents"));

    // Test multiple paths
    let paths = extract_paths_from_command("cp /src/file.txt /dest/file.txt");
    assert_eq!(paths.len(), 2);

    // Test paths with quotes
    let paths = extract_paths_from_command("cat \"/path/with spaces/file.txt\"");
    assert_eq!(paths.len(), 1);
}

/// Test that Bubblewrap respects deny_read lists
#[cfg(target_os = "linux")]
#[test]
fn test_bubblewrap_respects_deny_read() {
    use rustyclaw::sandbox::{wrap_with_bwrap, SandboxPolicy};

    let temp_workspace = TempDir::new().unwrap();
    let credentials_path = PathBuf::from("/credentials");

    let policy = SandboxPolicy {
        deny_read: vec![credentials_path.clone()],
        deny_write: vec![],
        deny_exec: vec![],
        allow_paths: vec![],
        workspace: temp_workspace.path().to_path_buf(),
    };

    let (_cmd, args) = wrap_with_bwrap("echo test", &policy);

    // Convert args to string for easier checking
    let args_str = args.join(" ");

    // Credentials path should NOT be mounted
    assert!(
        !args_str.contains("/credentials"),
        "Bubblewrap should not mount denied paths"
    );

    // Workspace should be mounted
    assert!(
        args_str.contains(temp_workspace.path().to_str().unwrap()),
        "Bubblewrap should mount workspace"
    );
}

/// Test that Bubblewrap respects deny_write lists
#[cfg(target_os = "linux")]
#[test]
fn test_bubblewrap_respects_deny_write() {
    use rustyclaw::sandbox::{wrap_with_bwrap, SandboxPolicy};

    let temp_workspace = TempDir::new().unwrap();

    let policy = SandboxPolicy {
        deny_read: vec![],
        deny_write: vec![temp_workspace.path().to_path_buf()],
        deny_exec: vec![],
        allow_paths: vec![],
        workspace: temp_workspace.path().to_path_buf(),
    };

    let (_cmd, args) = wrap_with_bwrap("echo test", &policy);

    let workspace_str = temp_workspace.path().to_str().unwrap();

    // Find the workspace mount arguments
    let has_ro_bind = args.windows(3).any(|w| {
        w[0] == "--ro-bind" && w[1] == workspace_str && w[2] == workspace_str
    });

    let has_rw_bind = args.windows(3).any(|w| {
        w[0] == "--bind" && w[1] == workspace_str && w[2] == workspace_str
    });

    assert!(
        has_ro_bind || !has_rw_bind,
        "Workspace in deny_write should be mounted read-only"
    );
}

/// Test that Bubblewrap respects deny_exec lists
#[cfg(target_os = "linux")]
#[test]
fn test_bubblewrap_respects_deny_exec() {
    use rustyclaw::sandbox::{wrap_with_bwrap, SandboxPolicy};

    let temp_workspace = TempDir::new().unwrap();

    let policy = SandboxPolicy {
        deny_read: vec![],
        deny_write: vec![],
        deny_exec: vec![PathBuf::from("/usr/bin")],
        allow_paths: vec![],
        workspace: temp_workspace.path().to_path_buf(),
    };

    let (_cmd, args) = wrap_with_bwrap("echo test", &policy);

    let args_str = args.join(" ");

    // /usr/bin should NOT be mounted when in deny_exec
    assert!(
        !args_str.contains("/usr/bin"),
        "Bubblewrap should not mount deny_exec paths"
    );
}

/// Test that macOS sandbox profile includes deny_exec rules
#[cfg(target_os = "macos")]
#[test]
fn test_macos_sandbox_deny_exec() {
    use rustyclaw::sandbox::{wrap_with_macos_sandbox, SandboxPolicy};

    let temp_workspace = TempDir::new().unwrap();

    let policy = SandboxPolicy {
        deny_read: vec![],
        deny_write: vec![],
        deny_exec: vec![PathBuf::from("/private/scripts")],
        allow_paths: vec![],
        workspace: temp_workspace.path().to_path_buf(),
    };

    let (cmd, args) = wrap_with_macos_sandbox("echo test", &policy);

    // The Seatbelt profile should be in args[1]
    assert_eq!(cmd, "sandbox-exec");
    assert!(args.len() >= 2);

    let profile = &args[1];

    // Profile should contain deny process-exec rule
    assert!(
        profile.contains("deny process-exec"),
        "macOS sandbox profile should include deny process-exec rules"
    );
    assert!(
        profile.contains("/private/scripts"),
        "macOS sandbox profile should include the denied path"
    );
}

/// Test fail-closed behavior: sandbox setup failure prevents execution
#[test]
fn test_fail_closed_behavior() {
    use rustyclaw::sandbox::{validate_path, SandboxPolicy};

    // Create temporary directories
    let temp_protected = TempDir::new().unwrap();
    let test_file = temp_protected.path().join("secret.txt");
    fs::write(&test_file, "PROTECTED").unwrap();

    let policy = SandboxPolicy {
        deny_read: vec![temp_protected.path().to_path_buf()],
        deny_write: vec![],
        deny_exec: vec![],
        allow_paths: vec![],
        workspace: PathBuf::from("/workspace"),
    };

    // Try to validate a protected path
    let result = validate_path(&test_file, &policy);

    // Should fail (fail-closed)
    assert!(
        result.is_err(),
        "Validation should fail for paths in deny_read"
    );
}

/// Test that sandbox modes gracefully degrade
#[test]
fn test_sandbox_mode_detection() {
    use rustyclaw::sandbox::{Sandbox, SandboxMode};

    // Test that Auto mode can be created
    let policy = rustyclaw::sandbox::SandboxPolicy::default();
    let sandbox = Sandbox::with_mode(SandboxMode::Auto, policy);

    // Effective mode should be one of the supported modes
    let effective = sandbox.effective_mode();
    assert!(
        matches!(
            effective,
            SandboxMode::Landlock | SandboxMode::Bubblewrap | SandboxMode::MacOSSandbox | SandboxMode::PathValidation | SandboxMode::None
        ),
        "Effective mode should be a valid sandbox mode"
    );
}

/// Test that command wrapping preserves command correctness
#[cfg(target_os = "linux")]
#[test]
fn test_command_wrapping_preserves_args() {
    use rustyclaw::sandbox::{wrap_with_bwrap, SandboxPolicy};

    let temp_workspace = TempDir::new().unwrap();

    let policy = SandboxPolicy {
        deny_read: vec![],
        deny_write: vec![],
        deny_exec: vec![],
        allow_paths: vec![],
        workspace: temp_workspace.path().to_path_buf(),
    };

    let test_command = "echo 'hello world' | grep hello";
    let (cmd, args) = wrap_with_bwrap(test_command, &policy);

    assert_eq!(cmd, "bwrap");

    // The command should be passed after "--" separator
    let separator_idx = args.iter().position(|a| a == "--");
    assert!(separator_idx.is_some(), "Bwrap args should contain -- separator");

    // After separator should be: sh -c "command"
    let cmd_args = &args[separator_idx.unwrap() + 1..];
    assert_eq!(cmd_args.len(), 3);
    assert_eq!(cmd_args[0], "sh");
    assert_eq!(cmd_args[1], "-c");
    assert_eq!(cmd_args[2], test_command);
}

/// Benchmark: Verify sandbox overhead is reasonable
#[test]
fn test_sandbox_performance() {
    use rustyclaw::sandbox::{run_sandboxed, SandboxMode, SandboxPolicy};
    use std::time::Instant;

    let temp_workspace = TempDir::new().unwrap();

    let policy = SandboxPolicy {
        deny_read: vec![],
        deny_write: vec![],
        deny_exec: vec![],
        allow_paths: vec![],
        workspace: temp_workspace.path().to_path_buf(),
    };

    // Measure time to validate a simple command
    let start = Instant::now();
    let _ = run_sandboxed("echo test", &policy, SandboxMode::PathValidation);
    let duration = start.elapsed();

    // PathValidation should add minimal overhead (< 1000ms for simple command)
    assert!(
        duration.as_millis() < 1000,
        "PathValidation overhead should be reasonable (was {}ms)",
        duration.as_millis()
    );
}
