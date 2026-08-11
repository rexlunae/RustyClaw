//! Isolation for tests that run the real `rustyclaw` binary.
//!
//! Each integration test binary compiles this module separately, so a helper
//! only some of them use reads as dead code in the rest.
#![allow(dead_code)]
//!
//!
//! Every CLI test spawns the shipped binary, and the binary reads a real
//! installation unless told otherwise. `exit_codes.rs` ran `gateway stop`
//! with the developer's own `HOME`, so the binary resolved `~/.rustyclaw`,
//! read the live PID file and sent SIGTERM to whatever it named — a
//! `cargo test --workspace` on a machine with a gateway running killed that
//! gateway. The assertion was `code >= 0`, so it passed either way; it
//! arguably passed *more* reliably for having succeeded.
//!
//! The helper lives here rather than in each test file because the same
//! function already existed in `e2e.rs` and the other three files did not
//! have it. One copy cannot drift from another.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A scratch directory to use as `HOME`, unique per test and per process.
///
/// Not cleaned up: these hold nothing but whatever the CLI decides to create
/// on a fresh install, and leaving them costs a few empty directories in the
/// temp dir while making a failed run inspectable.
pub fn scratch_home(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rustyclaw-cli-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// The binary, pointed at a scratch `HOME` and nothing else.
///
/// Setting `HOME` alone is not enough. `--config`, `--settings-dir`,
/// `--profile`, `--soul`, `--skills` and `--gateway` are all env-backed
/// globals, so a developer who exports any of them has the CLI read their
/// real installation instead of the empty directory the test just built.
///
/// Cleared in the harness rather than at each call site so a test added later
/// cannot forget — which is the failure this exists to prevent, not a
/// hypothetical one.
pub fn scratch_command(binary: &Path, home: &Path) -> Command {
    let mut cmd = Command::new(binary);
    cmd.env("HOME", home);
    for leaked in [
        "RUSTYCLAW_CONFIG",
        "RUSTYCLAW_SETTINGS_DIR",
        "RUSTYCLAW_PROFILE",
        "RUSTYCLAW_SOUL",
        "RUSTYCLAW_SKILLS",
        "RUSTYCLAW_GATEWAY",
    ] {
        cmd.env_remove(leaked);
    }
    cmd
}
