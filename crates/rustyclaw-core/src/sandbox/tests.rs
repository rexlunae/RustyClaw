//! Sandbox tests.

use super::platform::{run_unsandboxed, run_with_path_validation};
use super::*;

#[test]
fn test_capabilities_detect() {
    let caps = SandboxCapabilities::detect();
    // Should always have at least path validation
    assert!(
        caps.best_mode() != SandboxMode::None || caps.best_mode() == SandboxMode::PathValidation
    );
}

#[test]
fn test_policy_creation() {
    let policy = SandboxPolicy::protect_credentials(
        "/home/user/.rustyclaw/credentials",
        "/home/user/.rustyclaw/workspace",
    );

    assert_eq!(policy.deny_read.len(), 1);
    assert!(policy.deny_read[0].ends_with("credentials"));
}

#[test]
fn test_path_validation_denied() {
    let policy = SandboxPolicy::protect_credentials("/tmp/creds", "/tmp/workspace");
    std::fs::create_dir_all("/tmp/creds").ok();
    // Ensure the file exists so canonicalize works
    let _ = std::fs::write("/tmp/creds/secrets.json", "test");
    let result = validate_path(Path::new("/tmp/creds/secrets.json"), &policy);
    assert!(result.is_err());
}

#[test]
fn test_path_validation_allowed() {
    let policy =
        SandboxPolicy::protect_credentials("/tmp/test-creds-isolated", "/tmp/test-workspace");

    let result = validate_path(Path::new("/tmp/other/file.txt"), &policy);
    assert!(result.is_ok());
}

#[test]
fn test_sandbox_mode_parsing() {
    assert_eq!("none".parse::<SandboxMode>().unwrap(), SandboxMode::None);
    assert_eq!("auto".parse::<SandboxMode>().unwrap(), SandboxMode::Auto);
    assert_eq!(
        "bwrap".parse::<SandboxMode>().unwrap(),
        SandboxMode::Bubblewrap
    );
    assert_eq!(
        "macos".parse::<SandboxMode>().unwrap(),
        SandboxMode::MacOSSandbox
    );
}

#[test]
fn test_sandbox_status() {
    let policy = SandboxPolicy::default();
    let sandbox = Sandbox::new(policy);
    let status = sandbox.status();
    assert!(status.contains("Mode:"));
    assert!(status.contains("Available:"));
}

#[cfg(target_os = "linux")]
#[test]
fn test_bwrap_command_generation() {
    let policy = SandboxPolicy {
        workspace: PathBuf::from("/home/user/workspace"),
        ..Default::default()
    };

    let (cmd, args) = wrap_with_bwrap("ls -la", &policy);

    assert_eq!(cmd, "bwrap");
    assert!(args.contains(&"--unshare-all".to_string()));
    assert!(args.contains(&"ls -la".to_string()));
}

#[test]
fn test_run_unsandboxed() {
    let output = run_unsandboxed("echo hello").unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("hello"));
}

#[test]
fn test_extract_paths_absolute() {
    let paths = extract_paths_from_command("cat /etc/passwd");
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], PathBuf::from("/etc/passwd"));
}

#[test]
fn test_extract_paths_home() {
    let paths = extract_paths_from_command("cat ~/file.txt");
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], PathBuf::from("~/file.txt"));
}

#[test]
fn test_extract_paths_multiple() {
    let paths = extract_paths_from_command("cp /etc/hosts ~/backup/hosts");
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0], PathBuf::from("/etc/hosts"));
    assert_eq!(paths[1], PathBuf::from("~/backup/hosts"));
}

#[test]
fn test_extract_paths_quoted() {
    let paths = extract_paths_from_command("cat \"/path/with spaces/file\"");
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], PathBuf::from("/path/with spaces/file"));
}

#[test]
fn test_extract_paths_no_paths() {
    let paths = extract_paths_from_command("echo hello world");
    assert_eq!(paths.len(), 0);
}

#[test]
fn test_extract_paths_complex_command() {
    let paths = extract_paths_from_command("tar czf /backup/archive.tar.gz ~/documents");
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0], PathBuf::from("/backup/archive.tar.gz"));
    assert_eq!(paths[1], PathBuf::from("~/documents"));
}

#[test]
fn test_path_validation_blocks_credentials() {
    let policy = SandboxPolicy::protect_credentials("/tmp/test_creds", "/tmp/test_workspace");
    std::fs::create_dir_all("/tmp/test_creds").ok();
    // Ensure the file exists so canonicalize works
    let _ = std::fs::write("/tmp/test_creds/secret.txt", "test");
    // This should fail because /tmp/test_creds is protected
    let result = run_with_path_validation("cat /tmp/test_creds/secret.txt", &policy);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Access denied"));
}

#[test]
fn test_path_validation_allows_workspace() {
    let policy = SandboxPolicy::protect_credentials("/tmp/test_creds2", "/tmp/test_workspace2");
    std::fs::create_dir_all("/tmp/test_workspace2").ok();

    // This should succeed because /tmp/test_workspace2 is not protected
    let result = run_with_path_validation("echo hello > /tmp/test_workspace2/file.txt", &policy);
    // Note: This will likely fail with "command failed" but NOT "Access denied"
    // because the shell redirection happens before echo runs
    if let Err(e) = result {
        assert!(!e.contains("Access denied"));
    }
}
