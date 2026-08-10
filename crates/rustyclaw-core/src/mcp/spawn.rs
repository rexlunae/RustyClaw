//! Deciding how an MCP server process is launched.
//!
//! MCP servers are arbitrary commands named in config — `npx`, `uvx`, a local
//! binary — and the tools they expose are injected straight into the model's
//! tool list, indistinguishable from first-party ones. Before issue #230 they
//! also inherited the gateway's entire filesystem view and environment, so a
//! compromised third-party package (or a supply-chain attack on one) could
//! read the vault and reach anything on the network the gateway could.
//!
//! The plan is built as a value rather than by mutating a `Command` inline, so
//! what gets spawned can be asserted in a test instead of only observed by
//! running it.

use super::config::{McpSandbox, McpServerConfig};
use crate::sandbox::{SandboxCapabilities, SandboxPolicy, platform};
use std::path::{Path, PathBuf};

/// Environment variables a confined server keeps from the parent.
///
/// Everything else is dropped. The gateway's environment is where provider API
/// keys live, and "the MCP server can read our OpenAI key" is the same finding
/// as "the MCP server can read the vault" — inheriting the whole environment
/// hands it over without the server having to touch the disk at all.
const ENV_PASSTHROUGH: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "TERM", "TMPDIR"];

/// What to spawn, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpawnPlan {
    /// The program to execute — `bwrap` when confined, the server otherwise.
    pub program: String,
    /// Arguments, including the confinement flags when confined.
    pub args: Vec<String>,
    /// The environment to run with. Replaces the inherited one entirely when
    /// [`Self::clear_env`] is set.
    pub env: Vec<(String, String)>,
    /// Whether to drop the inherited environment first.
    pub clear_env: bool,
    /// Whether this plan is actually confined — for logging that does not lie.
    pub confined: bool,
    /// Why it is not confined, when it is not and the caller asked for it.
    pub unconfined_reason: Option<String>,
}

/// Why a platform cannot confine, or `None` if it can.
///
/// Split from [`plan_spawn`] so the reason can be phrased once and reported
/// both in the `Required` refusal and the `Auto` warning, rather than drifting
/// into two descriptions of the same situation.
fn confinement_unavailable(caps: &SandboxCapabilities) -> Option<String> {
    if cfg!(not(target_os = "linux")) {
        return Some(
            "process confinement for MCP servers is implemented with bubblewrap, \
             which is Linux-only"
                .to_string(),
        );
    }
    if !caps.bubblewrap {
        return Some("bubblewrap (bwrap) is not installed or not runnable".to_string());
    }
    if !caps.unprivileged_userns {
        return Some(
            "unprivileged user namespaces are disabled, so bubblewrap cannot \
             create a sandbox"
                .to_string(),
        );
    }
    None
}

/// Whether a confined server's working directory sits inside the vault.
///
/// Refused for every mode, `auto` included. The degrade-and-warn path is for
/// platforms that cannot confine — a capability gap the operator cannot close.
/// This is a misconfiguration, and running it unconfined would hand the least
/// trustworthy configuration imaginable — a server rooted inside the vault —
/// the gateway's whole filesystem and its inherited environment, API keys and
/// all, behind a WARN. The safe direction for a misconfiguration is to stop.
///
/// A workspace that merely *contains* the vault is not a conflict: that one is
/// masked, so the server keeps its directory and the vault stays unreadable.
fn vault_conflict(workspace: &Path, credentials: &Path) -> Option<String> {
    workspace.starts_with(credentials).then(|| {
        format!(
            "the working directory {} is inside the credentials directory. Point `cwd` \
             somewhere else; a confined server cannot be given the vault, and running it \
             unconfined would be worse.",
            workspace.display()
        )
    })
}

/// A per-user scratch directory for servers that declare no `cwd`.
///
/// Under the settings directory, deliberately **not** the system temp dir.
/// `bwrap_confinement_args` emits the workspace bind and then `--tmpfs /tmp`,
/// and bubblewrap applies mounts in argument order — so a workspace under
/// `/tmp` is hidden by the tmpfs that follows it, `--chdir` then targets a
/// path that does not exist in the namespace, and bwrap exits fatally. The
/// first version of this put the scratch directory in `/tmp`, which would
/// have stopped every server without a `cwd` from starting at all.
fn private_workspace(settings_dir: &Path) -> PathBuf {
    let dir = settings_dir.join("mcp-workspace");
    // Best effort: if this cannot be created, bwrap will fail loudly on the
    // bind rather than silently confining to nothing.
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Decide how to launch `config`.
///
/// `Err` means the server must not start at all — only reachable via
/// [`McpSandbox::Required`], because that is the setting that says "no sandbox,
/// no server".
pub(crate) fn plan_spawn(
    config: &McpServerConfig,
    caps: &SandboxCapabilities,
) -> Result<SpawnPlan, String> {
    let configured_env: Vec<(String, String)> = config
        .env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let unconfined = |reason: Option<String>| SpawnPlan {
        program: config.command.clone(),
        args: config.args.clone(),
        env: configured_env.clone(),
        // Unconfined servers keep the inherited environment, which is what
        // they had before #230. Clearing it here would break working setups
        // without confining anything in exchange.
        clear_env: false,
        confined: false,
        unconfined_reason: reason,
    };

    if config.sandbox == McpSandbox::Off {
        return Ok(unconfined(None));
    }

    if let Some(reason) = confinement_unavailable(caps) {
        return match config.sandbox {
            McpSandbox::Required => Err(reason),
            // The reason travels with the plan so the caller logs the specific
            // obstacle rather than a generic "not sandboxed".
            _ => Ok(unconfined(Some(reason))),
        };
    }

    // A server with no `cwd` gets a private directory, not the gateway's.
    //
    // The first version fell back to `std::env::current_dir()`, and the
    // workspace is bind-mounted read-write. Most configs — including the
    // examples in SANDBOX.md — set no `cwd`, so "confined" would have meant
    // read-write access to whatever directory the gateway was started in. If
    // that is the user's home, the server gets the vault: exactly the thing
    // issue #230 is about, handed over by the fix for it.
    let settings_dir = crate::runtime_ctx::get_settings_dir();
    let workspace = match (config.cwd.as_ref(), settings_dir.as_ref()) {
        (Some(cwd), _) => PathBuf::from(cwd),
        (None, Some(settings)) => private_workspace(settings),
        // Nowhere safe to put a scratch directory. Reported through the same
        // path as any other reason confinement cannot happen, so `auto` warns
        // and `required` refuses — rather than falling back to the gateway's
        // own directory, which is what this branch exists to avoid.
        (None, None) => {
            let reason = "no settings directory is registered, so there is nowhere to \
                          put a private workspace for a server with no `cwd`"
                .to_string();
            return match config.sandbox {
                McpSandbox::Required => Err(reason),
                _ => Ok(unconfined(Some(reason))),
            };
        }
    };

    // And the credentials directory is denied outright, the way every other
    // sandbox construction site in this crate does it (see
    // `tools::helpers::init_sandbox`). `SandboxPolicy::default()` carries
    // empty deny lists; using it here meant a configured `cwd` that happened
    // to be an ancestor of the vault exposed it.
    let policy = match settings_dir.as_ref() {
        Some(settings) => {
            let credentials = settings.join("credentials");
            SandboxPolicy::protect_credentials(credentials, &workspace)
        }
        // No settings directory registered: nothing to protect, and inventing
        // a path would deny something arbitrary.
        None => SandboxPolicy {
            workspace: workspace.clone(),
            ..SandboxPolicy::default()
        },
    };
    // A workspace *inside* something denied cannot be bound at all, so the
    // server would start somewhere other than where it was told to. The argv
    // builder falls back to the sandbox home rather than dying, but silently
    // relocating a server is not an improvement on saying so.
    if let Some(reason) = settings_dir
        .as_ref()
        .and_then(|settings| vault_conflict(&policy.workspace, &settings.join("credentials")))
    {
        return Err(reason);
    }

    let extra: Vec<PathBuf> = config.allow_paths.iter().map(PathBuf::from).collect();

    let (program, args) =
        platform::wrap_argv_with_bwrap(&config.command, &config.args, &policy, &extra);

    let mut env: Vec<(String, String)> = ENV_PASSTHROUGH
        .iter()
        .filter_map(|key| std::env::var(key).ok().map(|v| ((*key).to_string(), v)))
        .collect();
    // HOME was passed through pointing at the real home directory, which is
    // not mounted in the namespace. `npx` writes to `$HOME/.npm` and `uvx` to
    // `$HOME/.cache/uv`, so both failed on a path that does not exist —
    // including the example in SANDBOX.md. Point it at the tmpfs instead: the
    // caches are disposable, and a confined server has no business reading
    // the real home anyway.
    env.retain(|(k, _)| k != "HOME");
    env.push(("HOME".to_string(), crate::sandbox::SANDBOX_HOME.to_string()));
    env.retain(|(k, _)| k != "TMPDIR");
    env.push(("TMPDIR".to_string(), "/tmp".to_string()));
    // The server's own configuration wins over the inherited value, so a
    // server that sets its own HOME or PATH still gets it.
    for (key, value) in configured_env {
        env.retain(|(k, _)| k != &key);
        env.push((key, value));
    }

    Ok(SpawnPlan {
        program,
        args,
        env,
        clear_env: true,
        confined: true,
        unconfined_reason: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::collections::HashMap;

    fn capable() -> SandboxCapabilities {
        SandboxCapabilities {
            landlock: true,
            bubblewrap: true,
            macos_sandbox: false,
            unprivileged_userns: true,
            docker: false,
        }
    }

    fn incapable() -> SandboxCapabilities {
        SandboxCapabilities {
            bubblewrap: false,
            ..capable()
        }
    }

    fn server(sandbox: McpSandbox) -> McpServerConfig {
        McpServerConfig {
            command: "npx".to_string(),
            args: vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-time".to_string(),
            ],
            cwd: Some("/tmp/ws".to_string()),
            sandbox,
            ..McpServerConfig::default()
        }
    }

    /// `Off` is the pre-#230 behaviour and has to stay exactly that: the
    /// command runs as written, with the inherited environment.
    #[test]
    fn sandbox_off_spawns_the_command_as_written() {
        let plan = plan_spawn(&server(McpSandbox::Off), &capable()).expect("off never refuses");
        assert_eq!(plan.program, "npx");
        assert_eq!(plan.args, vec!["-y", "@modelcontextprotocol/server-time"]);
        assert!(!plan.confined);
        assert!(
            !plan.clear_env,
            "off must not change the environment either"
        );
    }

    /// "Confine or do not run" has to actually refuse, not warn and continue.
    /// A sandbox that quietly is not there is worse than none, because it is
    /// believed.
    #[test]
    fn sandbox_required_refuses_when_the_platform_cannot_confine() {
        let err = plan_spawn(&server(McpSandbox::Required), &incapable())
            .expect_err("required must refuse rather than downgrade");
        assert!(err.contains("bubblewrap"), "got: {err}");
    }

    /// Auto degrades, but never silently — the reason rides along so the log
    /// line names the obstacle.
    #[test]
    fn sandbox_auto_degrades_with_a_reason_rather_than_silently() {
        let plan = plan_spawn(&server(McpSandbox::Auto), &incapable()).expect("auto degrades");
        assert!(!plan.confined);
        let reason = plan.unconfined_reason.expect("a reason must be attached");
        assert!(reason.contains("bubblewrap"), "got: {reason}");
    }

    /// And when it can confine, it does, without asking.
    #[test]
    fn auto_is_confined_by_default_where_the_platform_allows_it() {
        assert_eq!(McpSandbox::default(), McpSandbox::Auto);
        if confinement_unavailable(&capable()).is_some() {
            // Non-Linux build: the platform genuinely cannot, and the test
            // above already covers that path.
            return;
        }
        let plan = plan_spawn(&server(McpSandbox::Auto), &capable()).expect("auto plans");
        assert!(plan.confined);
        assert_eq!(plan.program, "bwrap");
        assert!(plan.unconfined_reason.is_none());
    }

    /// The argv must reach bwrap split, not rendered back into a shell string
    /// — that rendering is where quoting bugs turn into injection.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_command_reaches_bwrap_as_argv_with_no_shell_in_between() {
        let mut cfg = server(McpSandbox::Auto);
        cfg.args = vec!["--flag".to_string(), "a b; rm -rf /".to_string()];
        let plan = plan_spawn(&cfg, &capable()).expect("plans");

        assert!(
            !plan.args.iter().any(|a| a == "-c"),
            "a shell appeared in the argv: {:?}",
            plan.args
        );
        let sep = plan
            .args
            .iter()
            .position(|a| a == "--")
            .expect("bwrap needs the -- separator");
        assert_eq!(
            &plan.args[sep + 1..],
            &[
                "npx".to_string(),
                "--flag".to_string(),
                "a b; rm -rf /".to_string()
            ],
            "arguments must arrive as separate argv entries"
        );
    }

    /// The 🟥 from review: with no `cwd`, the workspace fell back to the
    /// gateway's current directory — which bwrap binds **read-write**. If the
    /// gateway started in the user's home, "confined" meant read-write access
    /// to `~/.rustyclaw/credentials`, the exact thing #230 is about.
    ///
    /// No settings directory is registered under test, so this exercises the
    /// branch that refuses to invent a workspace. Asserting on the reason
    /// rather than on the absence of a path: an earlier version of this test
    /// checked only that the current directory was missing from the argv,
    /// which the unconfined plan satisfies trivially — it has no argv to
    /// speak of.
    #[test]
    fn a_server_without_a_cwd_never_falls_back_to_the_gateways_directory() {
        let mut cfg = server(McpSandbox::Auto);
        cfg.cwd = None;
        let plan = plan_spawn(&cfg, &capable()).expect("auto degrades rather than failing");

        assert!(!plan.confined, "confinement needs a workspace it can name");
        let reason = plan.unconfined_reason.expect("a reason must be attached");
        assert!(reason.contains("workspace"), "got: {reason}");

        let here = std::env::current_dir().expect("a current directory");
        assert!(
            !plan.args.iter().any(|a| a == &here.display().to_string()),
            "the gateway's own directory reached the plan: {:?}",
            plan.args
        );
    }

    /// And `required` refuses outright rather than running unconfined.
    #[test]
    fn required_refuses_when_there_is_nowhere_to_put_a_workspace() {
        let mut cfg = server(McpSandbox::Required);
        cfg.cwd = None;
        let err = plan_spawn(&cfg, &capable()).expect_err("required must refuse");
        assert!(err.contains("workspace"), "got: {err}");
    }

    /// bubblewrap applies mounts in order, so `--tmpfs /tmp` emitted after a
    /// bind under /tmp hid it, and the `--chdir` that followed died on a path
    /// absent from the namespace. A `cwd` under /tmp is ordinary.
    #[test]
    fn a_workspace_under_tmp_is_not_hidden_by_the_tmpfs() {
        let plan = plan_spawn(&server(McpSandbox::Auto), &capable()).expect("plans");
        let tmpfs = plan
            .args
            .windows(2)
            .position(|w| w[0] == "--tmpfs" && w[1] == "/tmp")
            .expect("a tmpfs over /tmp");
        let bind = plan
            .args
            .iter()
            .position(|a| a == "/tmp/ws")
            .expect("the workspace bind");
        assert!(
            bind > tmpfs,
            "the tmpfs hides the workspace: {:?}",
            plan.args
        );
    }

    /// npx writes to `$HOME/.npm` and uvx to `$HOME/.cache/uv`. HOME pointed
    /// at the real home directory, which is not mounted, so both failed —
    /// including the documented example.
    #[test]
    fn home_points_somewhere_that_exists_inside_the_sandbox() {
        let plan = plan_spawn(&server(McpSandbox::Auto), &capable()).expect("plans");
        let home = plan
            .env
            .iter()
            .find(|(k, _)| k == "HOME")
            .map(|(_, v)| v.clone())
            .expect("HOME is set");
        assert_eq!(home, "/tmp/home", "HOME must be on the writable tmpfs");
    }

    /// A `cwd` inside the vault is refused for **every** mode, `auto` too.
    ///
    /// The degrade-and-warn path exists for platforms that cannot confine — a
    /// gap the operator cannot close. Routing a misconfiguration down it meant
    /// the least trustworthy configuration imaginable, a server rooted inside
    /// the credentials directory, was the one that ran with the gateway's whole
    /// filesystem and inherited environment, API keys included, behind a WARN.
    #[test]
    fn a_cwd_inside_the_vault_is_a_conflict_whatever_the_mode() {
        let vault = Path::new("/home/user/.rustyclaw/credentials");

        let conflict = vault_conflict(Path::new("/home/user/.rustyclaw/credentials/x"), vault)
            .expect("a cwd inside the vault must conflict");
        assert!(
            conflict.contains("credentials directory"),
            "got: {conflict}"
        );
        assert!(
            vault_conflict(vault, vault).is_some(),
            "the vault itself is inside the vault"
        );

        // And the ordinary cases stay usable — including a workspace that
        // merely contains the vault, which is masked rather than refused.
        assert!(vault_conflict(Path::new("/home/user"), vault).is_none());
        assert!(vault_conflict(Path::new("/srv/app"), vault).is_none());
    }

    /// A server told about a directory gets that directory, and only it.
    #[cfg(target_os = "linux")]
    #[test]
    fn allow_paths_become_binds() {
        let mut cfg = server(McpSandbox::Auto);
        // A path that exists, so the bind is not skipped as missing.
        cfg.allow_paths = vec!["/usr/share".to_string()];
        let plan = plan_spawn(&cfg, &capable()).expect("plans");
        assert!(
            plan.args
                .windows(2)
                .any(|w| w[0] == "--ro-bind" && w[1] == "/usr/share"),
            "expected a read-only bind for the allowed path: {:?}",
            plan.args
        );
        // Read-only specifically. The doc comment said "read-only mounts"
        // while the code emitted `--bind`, so listing a directory in
        // allow_paths silently granted write and delete too.
        assert!(
            !plan
                .args
                .windows(2)
                .any(|w| w[0] == "--bind" && w[1] == "/usr/share"),
            "the allowed path was mounted writable: {:?}",
            plan.args
        );
        assert!(
            !plan.args.iter().any(|a| a == "/etc/shadow"),
            "nothing should be bound that was not asked for"
        );
    }

    /// The environment is where provider API keys live, so a confined server
    /// starts from a known small set rather than from whatever the gateway
    /// happened to be started with.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_confined_server_does_not_inherit_the_gateways_environment() {
        let plan = plan_spawn(&server(McpSandbox::Auto), &capable()).expect("plans");
        assert!(plan.clear_env, "the inherited environment must be dropped");
        for (key, _) in &plan.env {
            assert!(
                ENV_PASSTHROUGH.contains(&key.as_str()),
                "{key} was passed through without being on the list"
            );
        }
    }

    /// But its own configured variables still arrive, and override the
    /// inherited value rather than appearing twice.
    #[cfg(target_os = "linux")]
    #[test]
    fn configured_environment_survives_and_wins() {
        let mut cfg = server(McpSandbox::Auto);
        cfg.env = HashMap::from([
            ("API_TOKEN".to_string(), "secret".to_string()),
            ("HOME".to_string(), "/custom/home".to_string()),
        ]);
        let plan = plan_spawn(&cfg, &capable()).expect("plans");

        assert!(
            plan.env
                .contains(&("API_TOKEN".to_string(), "secret".to_string()))
        );
        let homes: Vec<&String> = plan
            .env
            .iter()
            .filter(|(k, _)| k == "HOME")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(
            homes,
            vec!["/custom/home"],
            "HOME should appear once, overridden"
        );
    }
}
