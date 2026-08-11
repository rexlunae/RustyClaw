//! Isolation for tests that run the real `rustyclaw` binary.
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

// Each integration test binary compiles this module separately, so a helper
// only some of them use reads as dead code in the rest.
#![allow(dead_code)]

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
    // `dirs::home_dir()` — which is what resolves the settings directory, and
    // so the PID file `gateway stop` reads — takes `HOME` only on Unix. On
    // Windows it reads the user profile, so `HOME` alone leaves a Windows
    // developer's real installation in reach and the stop test still kills
    // their gateway. The CI matrix builds Windows targets.
    cmd.env("USERPROFILE", home);
    // Nothing on the `gateway stop` path uses these, but `config_dir`,
    // `data_dir` and `cache_dir` are reached elsewhere in the tree (SSH
    // known_hosts, model caches, messenger state). Redirected here so the
    // harness is isolating by default rather than isolating for exactly the
    // one command that has already caused trouble.
    cmd.env("APPDATA", home);
    cmd.env("LOCALAPPDATA", home);
    cmd.env("XDG_CONFIG_HOME", home.join("config"));
    cmd.env("XDG_DATA_HOME", home.join("data"));
    cmd.env("XDG_CACHE_HOME", home.join("cache"));
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
