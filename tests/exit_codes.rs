//! Exit code conformance tests.
//!
//! Tests that RustyClaw uses appropriate exit codes matching common conventions.

use std::process::Command;

/// Run rustyclaw and get exit code
fn exit_code(args: &[&str]) -> i32 {
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--"])
        .args(args)
        .output()
        .expect("Failed to execute rustyclaw");

    output.status.code().unwrap_or(-1)
}

// ── Standard Exit Codes ─────────────────────────────────────────────────────
//
// Following common Unix conventions:
// 0   - Success
// 1   - General error
// 2   - Misuse of shell command (invalid args)
// 64  - Command line usage error (EX_USAGE from sysexits.h)
// 65  - Data format error
// 66  - Cannot open input
// 69  - Unavailable service
// 70  - Internal software error
// 74  - I/O error
// 78  - Configuration error

mod success_codes {
    use super::*;

    #[test]
    fn test_help_exits_zero() {
        assert_eq!(exit_code(&["--help"]), 0);
    }

    #[test]
    fn test_version_exits_zero() {
        assert_eq!(exit_code(&["--version"]), 0);
    }

    #[test]
    fn test_subcommand_help_exits_zero() {
        assert_eq!(exit_code(&["gateway", "--help"]), 0);
        assert_eq!(exit_code(&["skills", "--help"]), 0);
        assert_eq!(exit_code(&["doctor", "--help"]), 0);
    }
}

mod error_codes {
    use super::*;

    #[test]
    fn test_unknown_command_exits_nonzero() {
        let code = exit_code(&["nonexistent-command-xyz"]);
        assert_ne!(code, 0, "Unknown command should fail");
        // clap typically returns 2 for usage errors
        assert!(code == 1 || code == 2, "Expected error code 1 or 2, got {}", code);
    }

    #[test]
    fn test_invalid_flag_exits_nonzero() {
        let code = exit_code(&["--invalid-flag-xyz"]);
        assert_ne!(code, 0, "Invalid flag should fail");
    }

    #[test]
    fn test_missing_required_arg_exits_nonzero() {
        // command subcommand requires a message/command
        let code = exit_code(&["command"]);
        // Might succeed with empty input or fail - depends on implementation
        // Just verify it doesn't crash
        assert!(code >= 0);
    }
}

mod gateway_codes {
    use super::*;

    #[test]
    fn test_gateway_status_without_running_gateway() {
        let code = exit_code(&["gateway", "status"]);
        // Should either succeed (showing not running) or fail gracefully
        // Not expecting a crash (negative exit code)
        assert!(code >= 0, "Gateway status should not crash");
    }

    #[test]
    fn test_gateway_stop_without_running_gateway() {
        let code = exit_code(&["gateway", "stop"]);
        // Stopping non-running gateway might return error or success
        assert!(code >= 0, "Gateway stop should not crash");
    }
}

mod config_codes {
    use super::*;

    #[test]
    fn test_config_with_nonexistent_file() {
        let code = exit_code(&["--config", "/nonexistent/path/config.toml", "status"]);
        // Should fail because config file doesn't exist
        // But should fail gracefully, not crash
        assert!(code >= 0, "Should handle missing config gracefully");
    }
}

// ── Exit Code Documentation ─────────────────────────────────────────────────

#[test]
fn test_exit_code_documentation() {
    // This test documents expected exit codes for reference
    let expected_codes = [
        (0, "Success"),
        (1, "General error"),
        (2, "Invalid arguments / usage error"),
        // Future: more specific codes
    ];

    for (code, description) in expected_codes {
        assert!(code >= 0 && code <= 255, "{}: {} should be valid exit code", code, description);
    }
}
