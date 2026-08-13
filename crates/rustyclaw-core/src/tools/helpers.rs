//! Helper functions and global state for the tools system.

use crate::ignore::Ignore;
use crate::process_manager::{ProcessManager, SharedProcessManager};
use crate::sandbox::{Sandbox, SandboxError, SandboxMode, SandboxPolicy};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tracing::{debug, error, warn};

// ── Global process manager ──────────────────────────────────────────────────

/// Global process manager for background exec sessions.
static PROCESS_MANAGER: OnceLock<SharedProcessManager> = OnceLock::new();

/// Get the global process manager instance.
pub fn process_manager() -> &'static SharedProcessManager {
    PROCESS_MANAGER.get_or_init(|| Arc::new(Mutex::new(ProcessManager::new())))
}

// ── Global sandbox configuration ────────────────────────────────────────────

/// Global sandbox instance, initialized once at gateway startup.
static SANDBOX: OnceLock<Sandbox> = OnceLock::new();

/// Called once from the gateway to initialize the sandbox.
///
/// Once means once: the `OnceLock` refuses a second set, and that is now a
/// logged fact instead of a silent one — a caller hoping to "re-register"
/// the sandbox was getting a no-op. The workspace baked in here is only the
/// policy's default; every execution site substitutes the per-command
/// working directory into its copy of the policy, which is what lets turns
/// in different projects run under the right confinement at the same time.
pub fn init_sandbox(
    mode: SandboxMode,
    workspace: PathBuf,
    credentials_dir: PathBuf,
    deny_paths: Vec<PathBuf>,
) {
    debug!(?mode, ?workspace, "Initializing sandbox");
    let mut policy = SandboxPolicy::protect_credentials(&credentials_dir, &workspace);
    for path in deny_paths {
        policy = policy.deny_read(path.clone()).deny_write(path);
    }
    let sandbox = Sandbox::with_mode(mode, policy);
    if SANDBOX.set(sandbox).is_err() {
        warn!("Sandbox already initialized; ignoring re-initialization");
    }
}

/// Build the sandbox-wrapped `(program, args)` for running `interpreter` as a
/// long-lived child that reads its script from stdin, honoring the active
/// sandbox policy with the given working directory.
///
/// Returns `None` when no child-wrappable sandbox is active (no sandbox
/// configured, mode `None`/`PathValidation`, or in-process-only `Landlock`),
/// in which case the caller should spawn the interpreter directly. Used by
/// the gateway's trigger supervisor so sandboxed triggers run under the same
/// isolation as tool commands while still receiving their code over stdin
/// (never on disk, never in argv). Network and the workspace remain available
/// under the standard policy, so trigger callbacks keep working.
pub fn sandbox_wrap_interpreter(interpreter: &str, cwd: &Path) -> Option<(String, Vec<String>)> {
    let sb = SANDBOX.get()?;
    let mut policy = sb.policy.clone();
    policy.workspace = cwd.to_path_buf();
    match sb.effective_mode() {
        SandboxMode::Bubblewrap | SandboxMode::LandlockBwrap => {
            Some(crate::sandbox::wrap_with_bwrap(interpreter, &policy))
        }
        SandboxMode::MacOSSandbox => Some(crate::sandbox::wrap_with_macos_sandbox(
            interpreter,
            &policy,
        )),
        // None / PathValidation / Landlock(in-process) / Docker(no stdin
        // streaming) / Auto(already resolved): spawn directly.
        _ => None,
    }
}

/// Run a command through the sandbox (or unsandboxed if not initialized).
pub fn run_sandboxed_command(
    command: &str,
    cwd: &Path,
) -> Result<std::process::Output, SandboxError> {
    if let Some(sb) = SANDBOX.get() {
        debug!(mode = ?sb.mode, cwd = %cwd.display(), "Running sandboxed command");
        // Update policy workspace to the actual cwd for this command
        let mut policy = sb.policy.clone();
        policy.workspace = cwd.to_path_buf();
        crate::sandbox::run_sandboxed(command, &policy, sb.mode)
    } else {
        debug!(cwd = %cwd.display(), "Running unsandboxed command (no sandbox configured)");
        // No sandbox configured, run directly
        Ok(std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .output()?)
    }
}

// ── Credentials directory protection ────────────────────────────────────────

/// Absolute path of the credentials directory, set once at gateway startup.
static CREDENTIALS_DIR: OnceLock<PathBuf> = OnceLock::new();

// ── Global vault for cookie jar access ──────────────────────────────────────

use crate::secrets::SecretsManager;

/// Shared vault type for thread-safe access (uses tokio::sync::Mutex for async).
pub type SharedVault = Arc<tokio::sync::Mutex<SecretsManager>>;

/// Global vault instance, set once at gateway startup.
static VAULT: OnceLock<SharedVault> = OnceLock::new();

/// Called once from the gateway to register the vault for tool access.
pub fn set_vault(vault: SharedVault) {
    VAULT.set(vault).ignore();
}

/// Get the global vault instance, if initialized.
pub fn vault() -> Option<&'static SharedVault> {
    VAULT.get()
}

/// Called once from the gateway to register the credentials path.
pub fn set_credentials_dir(path: PathBuf) {
    CREDENTIALS_DIR.set(path).ignore();
}

/// Returns `true` when a command string references the credentials directory.
pub fn command_references_credentials(command: &str) -> bool {
    // A user-granted override (see `tools::guard_override`) stands down the
    // heuristic for the one retried call.
    if crate::tools::guard_override::is_granted() {
        return false;
    }
    if let Some(cred_dir) = CREDENTIALS_DIR.get() {
        let cred_str = cred_dir.to_string_lossy();
        command.contains(cred_str.as_ref())
    } else {
        false
    }
}

/// Returns `true` when `path` falls inside the credentials directory.
///
/// Uses double-canonicalize to detect symlink races (TOCTOU).
pub fn is_protected_path(path: &Path) -> bool {
    // A user-granted override (see `tools::guard_override`) stands down the
    // guard for the one retried call the user has seen and approved.
    if crate::tools::guard_override::is_granted() {
        return false;
    }
    if let Some(cred_dir) = CREDENTIALS_DIR.get() {
        let canon_cred = match cred_dir.canonicalize() {
            Ok(p) => p,
            Err(_) => return false,
        };

        let canon_path = match resolve_path_no_race(path) {
            Ok(p) => p,
            Err(_) => {
                // File may not exist yet (write_file). Fall back to raw check.
                return path.starts_with(cred_dir);
            }
        };

        canon_path.starts_with(&canon_cred)
    } else {
        false
    }
}

/// Try to resolve a path and double-canonicalize to detect symlink swaps.
///
/// Returns `Ok(canonical)` if the path resolves consistently twice,
/// or `Err` if the path changed between resolutions (possible symlink race).
pub fn resolve_path_no_race(path: &Path) -> Result<PathBuf, SandboxError> {
    // Unless the source file exists, there's nothing to check for races.
    if !path.exists() {
        // For non-existent paths, just canonicalize what we can of the parent,
        // then reattach the filename.
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => return Ok(path.to_path_buf()),
        };
        let filename = path
            .file_name()
            .map(|n| Path::new(n).to_path_buf())
            .unwrap_or_default();
        match parent.canonicalize() {
            Ok(canon_parent) => Ok(canon_parent.join(filename)),
            Err(_) => Ok(path.to_path_buf()),
        }
    } else {
        // Double-canonicalize to catch symlink swaps.
        let canon1 = path
            .canonicalize()
            .map_err(|e| SandboxError::PathResolution {
                path: path.to_path_buf(),
                source: e,
            })?;
        let canon2 = path
            .canonicalize()
            .map_err(|e| SandboxError::PathResolution {
                path: path.to_path_buf(),
                source: e,
            })?;

        if canon1 != canon2 {
            error!(
                path1 = %canon1.display(),
                path2 = %canon2.display(),
                "Path changed between resolutions — possible symlink race attack"
            );
            return Err(SandboxError::SymlinkRace);
        }

        Ok(canon2)
    }
}

/// Open a file for reading with O_NOFOLLOW on Linux and a final path-ownership check.
///
/// Returns `(File, canonical_path)` so the caller can use the fd without
/// worrying about the path changing under them.
pub fn open_file_read_safe(path: &Path) -> std::io::Result<(std::fs::File, PathBuf)> {
    // Step 1: resolve path safely, catching symlink races before opening.
    let canonical = resolve_path_no_race(path).map_err(std::io::Error::other)?;

    // Step 2: open with O_NOFOLLOW on Linux (fails if final component is a symlink).
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;

        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&canonical)?;

        // Step 3: verify the opened fd still points where we expect.
        let fd_path = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
        if fd_path != canonical {
            return Err(std::io::Error::other(format!(
                "Symlink race detected: opened fd points to {}, expected {}",
                fd_path.display(),
                canonical.display()
            )));
        }

        Ok((file, canonical))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let file = std::fs::File::open(&canonical)?;
        Ok((file, canonical))
    }
}

/// Open a file for writing with O_NOFOLLOW on Linux and TOCTOU protection.
pub fn open_file_write_safe(path: &Path) -> std::io::Result<(std::fs::File, PathBuf)> {
    // For writes, the file may not exist yet — resolve what we can.
    let canonical = if path.exists() {
        resolve_path_no_race(path).map_err(std::io::Error::other)?
    } else {
        // File doesn't exist — canonicalize the parent directory.
        let parent = path.parent().unwrap_or(Path::new("."));
        let filename = path
            .file_name()
            .map(|n| Path::new(n).to_path_buf())
            .unwrap_or_default();
        let canon_parent = resolve_path_no_race(parent).map_err(std::io::Error::other)?;
        canon_parent.join(filename)
    };

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;

        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&canonical)?;

        let fd_path = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
        if fd_path != canonical {
            return Err(std::io::Error::other(format!(
                "Symlink race detected: opened fd points to {}, expected {}",
                fd_path.display(),
                canonical.display()
            )));
        }

        Ok((file, canonical))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&canonical)?;
        Ok((file, canonical))
    }
}

/// Standard denial message when a tool tries to touch the vault.
pub const VAULT_ACCESS_DENIED: &str = "Access denied: the credentials directory is protected. Use the secrets_list / secrets_get / secrets_store tools instead.";

// ── Path helpers ────────────────────────────────────────────────────────────

/// Resolve a path argument against the workspace root.
/// Absolute paths are used as-is; relative paths are joined to `workspace_dir`.
pub fn resolve_path(workspace_dir: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_dir.join(p)
    }
}

/// Expand a leading `~` to the user's home directory.
///
/// Only the current user's home is expanded: `~alice/notes` is returned as-is,
/// because another account's home directory is not ours to guess and treating
/// `alice` as a subdirectory of *our* home would silently point at the wrong
/// place. A path with no leading `~`, or a `~` we cannot resolve a home for,
/// is likewise handed back untouched.
pub fn expand_tilde(p: &str) -> PathBuf {
    let Some(rest) = p.strip_prefix('~') else {
        return PathBuf::from(p);
    };
    let separated = rest.starts_with('/') || (cfg!(windows) && rest.starts_with('\\'));
    if !(rest.is_empty() || separated) {
        return PathBuf::from(p);
    }
    let Some(home) = dirs::home_dir() else {
        return PathBuf::from(p);
    };
    let rest = rest.trim_start_matches(['/', '\\']);
    if rest.is_empty() {
        home
    } else {
        home.join(rest)
    }
}

/// Blocked shell metacharacter patterns for command validation.
///
/// `${HOME` used to be here, as a way of catching `${HOME}/.rustyclaw/…`
/// spellings that the literal `~/.rustyclaw` check missed. It blocked every
/// ordinary use of the variable with it — `cd ${HOME}/src && cargo build` was
/// refused — and it is no longer needed: the settings-directory rule below
/// matches on the path's leaf, so it sees `${HOME}` spellings for free.
const BLOCKED_COMMAND_PATTERNS: &[&str] = &["$(<", "${cred", "${CRED"];

/// Blocked substrings in command strings indicating credential access attempts.
///
/// These are matched anywhere in the command, because a file with one of
/// these names is credential material wherever it lives.
const BLOCKED_CRED_SUBSTRINGS: &[&str] = &[
    "/secrets.json",
    "/secrets.key",
    "/authorized_clients",
    "/client_ed25519_key",
    "/credentials/",
    "/.openclaw/",
];

/// Files inside a RustyClaw settings directory that hold credential material.
///
/// The settings directory as a whole is *not* sensitive — it holds the log
/// the agent may need to read, `boot.toml`, threads, projects and skills — so
/// only these leaf names are refused.
///
/// `config.toml` is on the list because it can still carry secrets in
/// plaintext: `MessengerConfig` has `token`, `password` and `access_token`
/// fields, and installs that predate the move to vault `secret_refs` keep
/// live bot tokens there. It comes off this list the day those fields do.
const SENSITIVE_SETTINGS_FILES: &[&str] = &[
    "config.toml",
    "ssh_host_key",
    "client_ed25519_key",
    "authorized_clients",
];

/// Split a command into path-like tokens on whitespace, shell separators and
/// quotes.
///
/// Matching on tokens rather than raw substrings is what lets the checks
/// below look at a path's *leaf*. `/proc/meminfo` and `/proc/<pid>/mem` share
/// the substring `/mem`; only one of them is a way to read another process's
/// memory, and the difference is visible only once the token is split out and
/// its last component examined.
fn command_tokens(command: &str) -> impl Iterator<Item = &str> {
    command
        .split(|c: char| {
            c.is_whitespace() || matches!(c, ';' | '|' | '&' | '(' | ')' | '<' | '>' | '"' | '\'')
        })
        .filter(|t| !t.is_empty())
}

/// The last path component of a token, or `""` for a token ending in `/`.
fn path_leaf(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or("")
}

/// Check a command string for direct credential-exfiltration patterns.
///
/// A heuristic pre-filter, not the enforcement boundary: the credentials
/// directory is denied at the OS level by
/// [`SandboxPolicy::protect_credentials`](crate::sandbox::SandboxPolicy)
/// (bubblewrap, Landlock, or seatbelt depending on platform), and the file
/// tools refuse protected paths through [`is_protected_path`]. This catches
/// shell spellings before the command runs, so precision matters: a rule that
/// refuses ordinary work trains the agent to give up, and an agent that
/// stops after three blocked commands is not more secure than one that reads
/// its own log.
///
/// Returns `true` if the command should be blocked.
pub fn command_has_exfiltration_patterns(command: &str) -> bool {
    let lower = command.to_lowercase();

    // Process introspection. `/proc/<pid>/mem` and `/proc/<pid>/environ` read
    // another process's memory and environment; the gateway is handed
    // `RUSTYCLAW_VAULT_PASSWORD` and `RUSTYCLAW_MODEL_API_KEY` through its
    // environment (see `daemon::start`), so `environ` is the most direct
    // route to both — and it was not previously blocked at all, while
    // `/proc/meminfo` was.
    for token in command_tokens(&lower) {
        if !token.contains("/proc/") {
            continue;
        }
        if matches!(path_leaf(token), "mem" | "environ") || token.contains("/fd/") {
            return true;
        }
    }

    // Block sensitive patterns.
    for &pat in BLOCKED_COMMAND_PATTERNS {
        if command.contains(pat) {
            return true;
        }
    }

    // Block known sensitive file patterns.
    for &pat in BLOCKED_CRED_SUBSTRINGS {
        if command.contains(pat) {
            return true;
        }
    }

    // Credential material inside a settings directory. Both halves are
    // checked across the whole command rather than within one token, so
    // `cd ~/.rustyclaw && cat config.toml` is caught as well as the single
    // path spelling. `.rustyclaw` rather than `.rustyclaw/` so that
    // `--profile` directories (`~/.rustyclaw-work`) match too.
    if lower.contains(".rustyclaw")
        && command_tokens(&lower).any(|t| SENSITIVE_SETTINGS_FILES.contains(&path_leaf(t)))
    {
        return true;
    }

    // Block `ln -s` that target protected dirs
    if lower.starts_with("ln -s") || lower.contains("; ln -s") || lower.contains("&& ln -s") {
        if let Some(cred_dir) = CREDENTIALS_DIR.get() {
            let cred_str = cred_dir.to_string_lossy().to_lowercase();
            if lower.contains(&cred_str) {
                return true;
            }
        }
    }

    false
}

/// Validate a command string for basic safety.
pub fn validate_command_safe(command: &str) -> Result<(), SandboxError> {
    // Null bytes are always blocked.
    if command.contains('\0') {
        return Err(SandboxError::CommandNullByte);
    }

    // Check command length.
    if command.len() > 4096 {
        return Err(SandboxError::CommandTooLong);
    }

    // Check for credential exfiltration patterns. A user-granted override
    // (see `tools::guard_override`) stands this heuristic down for the one
    // retried call; the null-byte and length checks above are correctness,
    // not policy, and hold regardless.
    if !crate::tools::guard_override::is_granted() && command_has_exfiltration_patterns(command) {
        return Err(SandboxError::CommandExfiltration);
    }

    Ok(())
}

/// Whether a tool-result message is one of the exfiltration-guard blocks
/// that a user may override (see `tools::guard_override` and issue #418).
///
/// Matched against the guards' own message constants, so it cannot drift
/// from what the guards actually say. Deliberately narrow: sandbox path
/// denials and vault access policy refusals are different mechanisms with
/// different override stories, and are not included.
pub fn is_guard_block(message: &str) -> bool {
    message.starts_with(VAULT_ACCESS_DENIED)
        || message.contains(&SandboxError::CommandExfiltration.to_string())
}

/// Redact sensitive HTTP header values from web fetch results.
/// Currently strips Authorization, Cookie, Set-Cookie, and X-API-Key headers.
#[allow(dead_code)]
pub fn redact_sensitive_headers(headers: &str) -> String {
    let mut result = String::new();
    for line in headers.lines() {
        if line.to_lowercase().starts_with("authorization:")
            || line.to_lowercase().starts_with("cookie:")
            || line.to_lowercase().starts_with("set-cookie:")
            || line.to_lowercase().starts_with("x-api-key:")
        {
            let colon_idx = line.find(':').unwrap_or(0);
            let header_name = &line[..=colon_idx];
            result.push_str(&format!("{} [REDACTED]\n", header_name));
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    result.trim_end().to_string()
}
/// If `found` lives inside `workspace_dir`, return a workspace-relative path
/// so the model can pass it directly to `read_file` (which will resolve it
/// back against `workspace_dir`).  Otherwise return the **absolute** path so
/// the model can still use it with tools that accept absolute paths.
pub fn display_path(found: &Path, workspace_dir: &Path) -> String {
    if let Ok(rel) = found.strip_prefix(workspace_dir) {
        rel.display().to_string()
    } else {
        found.display().to_string()
    }
}

/// Filter for `walkdir` — skip common non-content directories.
pub fn should_visit(entry: &walkdir::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    if entry.file_type().is_dir() {
        if matches!(
            name.as_ref(),
            ".git" | "node_modules" | "target" | ".hg" | ".svn" | "__pycache__" | "dist" | "build"
        ) {
            return false;
        }
        // Never recurse into the credentials directory.
        if is_protected_path(entry.path()) {
            return false;
        }
        true
    } else {
        true
    }
}

// ── Tool output sanitization ────────────────────────────────────────────────

/// Maximum size for tool output before truncation (50 KB).
const MAX_TOOL_OUTPUT_BYTES: usize = 50_000;

/// Detect if content looks like HTML or encoded binary data.
fn is_likely_garbage(s: &str) -> bool {
    // Check for HTML markers
    let lower = s.to_lowercase();
    if lower.contains("<!doctype") || lower.contains("<html") {
        return true;
    }

    // Check for base64-encoded data URIs
    if s.contains("data:image/") || s.contains("data:application/") {
        return true;
    }

    // Check for excessive base64-like content (long strings without spaces)
    let lines: Vec<&str> = s.lines().collect();
    let long_dense_lines = lines
        .iter()
        .filter(|line| line.len() > 500 && !line.contains(' '))
        .count();
    if long_dense_lines > 3 {
        return true;
    }

    false
}

/// Sanitize tool output: truncate if too large, warn if garbage detected.
pub fn sanitize_tool_output(output: String) -> String {
    // Check for garbage content first
    if is_likely_garbage(&output) {
        let preview_len = output.len().min(500);
        let preview: String = output.chars().take(preview_len).collect();
        warn!(bytes = output.len(), "Tool returned HTML/binary content");
        return format!(
            "[Warning: Tool returned HTML/binary content ({} bytes) — likely not useful]\n\nPreview:\n{}...",
            output.len(),
            preview
        );
    }

    // Truncate if too large
    if output.len() > MAX_TOOL_OUTPUT_BYTES {
        debug!(
            bytes = output.len(),
            max = MAX_TOOL_OUTPUT_BYTES,
            "Truncating large tool output"
        );
        let truncated: String = output.chars().take(MAX_TOOL_OUTPUT_BYTES).collect();
        format!(
            "{}...\n\n[Truncated: {} bytes total, showing first {}]",
            truncated,
            output.len(),
            MAX_TOOL_OUTPUT_BYTES
        )
    } else {
        output
    }
}

#[cfg(test)]
mod expand_tilde_tests {
    use super::expand_tilde;
    use std::path::PathBuf;

    #[test]
    fn expands_only_the_current_users_home() {
        let home = dirs::home_dir().expect("test host has a home directory");
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("~/code/app"), home.join("code/app"));
        assert_eq!(expand_tilde("/srv/api"), PathBuf::from("/srv/api"));
        assert_eq!(expand_tilde(""), PathBuf::from(""));
        // `~alice/notes` is another account's home. Expanding it against *our*
        // home would quietly resolve to `$HOME/alice/notes`, a path the caller
        // never asked for; leaving it alone fails visibly instead.
        assert_eq!(expand_tilde("~alice/notes"), PathBuf::from("~alice/notes"));
    }
}

#[cfg(test)]
mod command_guard_tests {
    use super::{command_has_exfiltration_patterns, path_leaf};

    // ── Command exfiltration heuristic ──────────────────────────────────
    //
    // This function had no tests, which is how three rules that refuse
    // ordinary work survived: the whole settings directory, every `${HOME}`
    // expansion, and `/proc/meminfo`. The two halves are kept as separate
    // tests so a future tightening that breaks legitimate commands, or a
    // loosening that lets credential material through, fails on the half it
    // actually broke.

    /// Commands an agent runs in the course of ordinary work. Every one of
    /// these was refused before; a `false` here is the bug the user reported
    /// as "running into this way too often".
    #[test]
    fn ordinary_commands_are_not_mistaken_for_exfiltration() {
        for cmd in [
            // The settings directory is not itself a secret. Reading the
            // gateway's own log is the first thing to do when it misbehaves.
            "ls -la ~/.rustyclaw/",
            "tail -50 ~/.rustyclaw/logs/gateway.log",
            "cat ~/.rustyclaw/boot.toml",
            "wc -l ~/.rustyclaw/threads/12.log.jsonl",
            // `${HOME}` is how you write a path in a shell.
            "cd ${HOME}/src && cargo build",
            "echo ${HOME}/projects",
            // Machine stats share a prefix with process introspection.
            "cat /proc/meminfo",
            "cat /proc/cpuinfo",
            "head -1 /proc/loadavg",
            // Nothing to do with credentials at all.
            "cargo test --workspace",
            "git log --oneline -20",
        ] {
            assert!(
                !command_has_exfiltration_patterns(cmd),
                "ordinary command was blocked: {cmd}"
            );
        }
    }

    /// The cases the guard exists for. `/proc/<pid>/environ` is new: the
    /// gateway receives the vault password and the model API key through its
    /// environment, so it is the most direct route to both, and the previous
    /// `contains("mem")` rule let it through while refusing `/proc/meminfo`.
    #[test]
    fn credential_access_is_still_blocked() {
        for cmd in [
            "cat /proc/self/environ",
            "cat /proc/1234/environ",
            "cat /proc/1234/mem",
            "ls /proc/self/fd/",
            "cat ~/.rustyclaw/credentials/openai",
            "cat ~/.rustyclaw/secrets.json",
            "cat ~/.rustyclaw/secrets.key",
            "cat ~/.rustyclaw/ssh_host_key",
            "cat ~/.rustyclaw/client_ed25519_key",
            "cat ~/.openclaw/credentials/anthropic",
            // config.toml can still hold plaintext messenger tokens.
            "cat ~/.rustyclaw/config.toml",
            "grep token ~/.rustyclaw/config.toml",
            // Spellings that avoid the literal `~/`.
            "cat $HOME/.rustyclaw/config.toml",
            "cat ${HOME}/.rustyclaw/config.toml",
            "cat /home/someone/.rustyclaw/config.toml",
            // A `--profile` settings directory is still a settings directory.
            "cat ~/.rustyclaw-work/config.toml",
            // Split across the command rather than in one path.
            "cd ~/.rustyclaw && cat config.toml",
            // Substitution tricks the metacharacter list exists for.
            "$(< ~/.somewhere/key)",
        ] {
            assert!(
                command_has_exfiltration_patterns(cmd),
                "credential access was allowed: {cmd}"
            );
        }
    }

    /// A project's own `config.toml` is not the settings directory's, and is
    /// refused only when the command also reaches for a settings directory.
    #[test]
    fn a_project_config_is_not_the_settings_config() {
        assert!(!command_has_exfiltration_patterns("cat ./config.toml"));
        assert!(!command_has_exfiltration_patterns("cat src/config.toml"));
    }

    #[test]
    fn path_leaf_handles_trailing_slashes_and_bare_names() {
        assert_eq!(path_leaf("~/.rustyclaw/config.toml"), "config.toml");
        assert_eq!(path_leaf("~/.rustyclaw/"), "");
        assert_eq!(path_leaf("config.toml"), "config.toml");
        assert_eq!(path_leaf(""), "");
    }
}
