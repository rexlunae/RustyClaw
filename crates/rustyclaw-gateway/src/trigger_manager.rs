//! External-trigger supervision and the fire callback endpoint.
//!
//! For every enabled [`TriggerDef`] the manager runs the trigger's code as a
//! child process (fed to the interpreter over stdin, never written to disk)
//! for as long as the gateway runs. Each process gets a random bearer token
//! and the address of
//! a localhost-only HTTP endpoint via environment variables; POSTing
//! `{"token": "...", "context": {...}}` to `http://<endpoint>/fire` makes
//! the gateway run the trigger's target agent with that context (see
//! [`crate::trigger_dispatch`]).
//!
//! Trigger processes run inside the sandbox layer by default (opt out per
//! trigger via `TriggerDef::sandboxed`); the sandbox keeps network access so
//! callbacks work and binds the target agent's workspace.
//!
//! The manager re-syncs against the encrypted store when woken through
//! [`rustyclaw_core::runtime_ctx::notify_triggers_changed`] (trigger tools
//! call it after every edit) and on a periodic tick, so definition changes
//! take effect while the gateway runs. All children are stopped when the
//! gateway shuts down.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rustyclaw_core::triggers::{
    ENV_TRIGGER_AGENT, ENV_TRIGGER_ENDPOINT, ENV_TRIGGER_ID, ENV_TRIGGER_TOKEN, TriggerStore,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// A validated fire request, handed to the dispatch task.
#[derive(Debug, Clone)]
pub struct TriggerFire {
    pub trigger_id: String,
    pub agent_id: String,
    pub context: serde_json::Value,
}

/// Minimum seconds between respawns of a crashing trigger process.
const RESPAWN_COOLDOWN: Duration = Duration::from_secs(10);
/// Periodic re-sync / reap interval.
const SYNC_INTERVAL: Duration = Duration::from_secs(15);
/// Upper bound on a fire request (headers + body).
const MAX_REQUEST_BYTES: usize = 256 * 1024;

/// token → (trigger_id, agent_id), rebuilt as processes start and stop.
type TokenMap = Arc<Mutex<HashMap<String, (String, String)>>>;

struct RunningTrigger {
    child: tokio::process::Child,
    fingerprint: u64,
    token: String,
    /// Set once the child is observed to have exited. A tombstoned entry
    /// (`Some`) is retained only to hold back its respawn until the cooldown
    /// elapses; its child status is never probed again, so this does not
    /// depend on `try_wait` succeeding twice. Time since this instant gates
    /// the respawn.
    exited_at: Option<Instant>,
}

/// Run the trigger subsystem until `cancel` fires. Spawned once by the
/// gateway's standalone listen loop.
pub async fn run_trigger_manager(
    settings_dir: PathBuf,
    workspace_root: PathBuf,
    fire_tx: mpsc::Sender<TriggerFire>,
    notify: Arc<Notify>,
    cancel: CancellationToken,
) -> Result<()> {
    let store = TriggerStore::open(&settings_dir);
    let tokens: TokenMap = Arc::new(Mutex::new(HashMap::new()));

    // Localhost-only callback endpoint on an ephemeral port.
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("Failed to bind trigger callback endpoint")?;
    let endpoint = listener
        .local_addr()
        .context("Failed to read trigger endpoint address")?;
    info!(endpoint = %endpoint, "Trigger callback endpoint listening (localhost only)");

    // Accept loop for fire callbacks.
    let accept_tokens = tokens.clone();
    let accept_cancel = cancel.clone();
    let accept_fire_tx = fire_tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = accept_cancel.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((stream, peer)) = accepted else { continue };
                    let tokens = accept_tokens.clone();
                    let fire_tx = accept_fire_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_fire_connection(stream, tokens, fire_tx).await {
                            debug!(peer = %peer, error = %e, "Trigger callback error");
                        }
                    });
                }
            }
        }
    });

    let mut running: HashMap<String, RunningTrigger> = HashMap::new();
    // While any trigger is in its respawn cooldown, tick fast enough to
    // respawn it promptly once the cooldown lapses; otherwise idle.
    let idle_interval = SYNC_INTERVAL;
    let cooldown_interval = (RESPAWN_COOLDOWN / 2).max(Duration::from_secs(1));

    loop {
        sync(endpoint, &workspace_root, &store, &mut running, &tokens).await;
        let interval = if running.values().any(|rt| rt.exited_at.is_some()) {
            cooldown_interval
        } else {
            idle_interval
        };
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = notify.notified() => {
                debug!("Trigger store changed — re-syncing");
            }
            _ = tokio::time::sleep(interval) => {}
        }
    }

    // Shutdown: stop every trigger process with the gateway.
    info!(count = running.len(), "Stopping trigger processes…");
    for (id, mut rt) in running.drain() {
        let _ = rt.child.start_kill();
        let _ = rt.child.wait().await;
        debug!(trigger = %id, "Trigger process stopped");
    }
    Ok(())
}

/// Reconcile running processes against the store: start new/changed enabled
/// triggers, stop removed/disabled ones, respawn crashed ones (rate-limited).
async fn sync(
    endpoint: std::net::SocketAddr,
    workspace_root: &std::path::Path,
    store: &TriggerStore,
    running: &mut HashMap<String, RunningTrigger>,
    tokens: &TokenMap,
) {
    let defs = match store.list() {
        Ok(defs) => defs,
        Err(e) => {
            warn!(error = %e, "Failed to load trigger store");
            return;
        }
    };
    let wanted: HashMap<&str, _> = defs
        .iter()
        .filter(|d| d.enabled)
        .map(|d| (d.id.as_str(), d))
        .collect();

    // Stop processes whose definition is gone, disabled, or changed.
    let stale: Vec<String> = running
        .iter_mut()
        .filter_map(|(id, rt)| {
            let changed = wanted
                .get(id.as_str())
                .map(|d| d.fingerprint() != rt.fingerprint);
            match changed {
                None => Some(id.clone()),       // removed or disabled
                Some(true) => Some(id.clone()), // definition changed
                Some(false) => None,
            }
        })
        .collect();
    for id in stale {
        if let Some(mut rt) = running.remove(&id) {
            let _ = rt.child.start_kill();
            let _ = rt.child.wait().await;
            tokens.lock().await.remove(&rt.token);
            info!(trigger = %id, "Trigger process stopped (definition removed/changed)");
        }
    }

    // Detect newly-exited processes exactly once. `try_wait` is only ever
    // probed while `exited_at` is None, so we never depend on it succeeding
    // twice for the same child (which it does not on Unix — the second
    // `waitpid` returns ECHILD). A tombstone is left in `running` with
    // `exited_at` set; its token is withdrawn immediately so a lingering
    // process can no longer fire.
    let mut newly_exited: Vec<(String, String)> = Vec::new();
    for (id, rt) in running.iter_mut() {
        if rt.exited_at.is_none() {
            if let Ok(Some(status)) = rt.child.try_wait() {
                warn!(trigger = %id, status = %status, "Trigger process exited");
                rt.exited_at = Some(Instant::now());
                newly_exited.push((id.clone(), rt.token.clone()));
            }
        }
    }
    if !newly_exited.is_empty() {
        let mut tok = tokens.lock().await;
        for (_, token) in &newly_exited {
            tok.remove(token);
        }
    }

    // Drop tombstones whose cooldown has lapsed so the start loop respawns
    // them fresh. Time since exit gates the respawn, not a re-probe of the
    // (already-reaped) child.
    let ready: Vec<String> = running
        .iter()
        .filter_map(|(id, rt)| {
            rt.exited_at
                .filter(|t| t.elapsed() >= RESPAWN_COOLDOWN)
                .map(|_| id.clone())
        })
        .collect();
    for id in ready {
        debug!(trigger = %id, "Respawn cooldown elapsed");
        running.remove(&id);
    }

    // Start anything enabled that isn't running (or tombstoned).
    for (id, def) in wanted {
        if running.contains_key(id) {
            continue;
        }
        match spawn_trigger(def, endpoint, workspace_root).await {
            Ok((child, token)) => {
                tokens
                    .lock()
                    .await
                    .insert(token.clone(), (def.id.clone(), def.agent_id.clone()));
                running.insert(
                    def.id.clone(),
                    RunningTrigger {
                        child,
                        fingerprint: def.fingerprint(),
                        token,
                        exited_at: None,
                    },
                );
                info!(trigger = %id, agent = %def.agent_id, "Trigger process started");
            }
            Err(e) => warn!(trigger = %id, error = %e, "Failed to start trigger process"),
        }
    }
}

/// Spawn a trigger's process, feeding its code to the interpreter over
/// stdin.
///
/// The code is piped to the interpreter's standard input rather than written
/// to a script file, so trigger code — which often embeds API keys or webhook
/// secrets — is never materialized as plaintext on disk, and never appears in
/// the process's argv (visible via `ps` / `/proc`). Shells (`sh`/`bash`/…),
/// `python3`, `node`, `ruby`, `perl`, etc. all execute a program read from a
/// non-tty stdin, which is the trigger contract.
async fn spawn_trigger(
    def: &rustyclaw_core::triggers::TriggerDef,
    endpoint: std::net::SocketAddr,
    workspace_root: &std::path::Path,
) -> Result<(tokio::process::Child, String)> {
    // Validate the interpreter command through the same safety check the
    // managed-services supervisor uses before spawning any subprocess
    // (STYLE_GUIDE.md: shell/subprocess execution goes through the sandbox
    // layer).
    rustyclaw_core::tools::validate_command_safe(def.interpreter())
        .map_err(|e| anyhow::anyhow!("Trigger interpreter rejected: {}", e))?;

    // Sandbox cwd is the target agent's workspace (mirrors Config layout:
    // `main` uses the installation workspace, others get a private one).
    let workspace = if def.agent_id == rustyclaw_core::agents::MAIN_AGENT_ID {
        workspace_root.to_path_buf()
    } else {
        workspace_root
            .parent()
            .map(|p| p.join("agents").join(&def.agent_id).join("workspace"))
            .unwrap_or_else(|| workspace_root.to_path_buf())
    };

    // Sandboxed triggers run the interpreter under the isolation wrapper
    // (bwrap / macOS seatbelt) with the code still delivered over stdin.
    // When no child-wrappable sandbox is active, or the trigger opted out,
    // spawn the interpreter directly.
    let wrapped = if def.sandboxed {
        rustyclaw_core::tools::sandbox_wrap_interpreter(def.interpreter(), &workspace)
    } else {
        None
    };
    let (program, args): (String, Vec<String>) = match wrapped {
        Some((prog, args)) => (prog, args),
        None => {
            if def.sandboxed {
                debug!(
                    trigger = %def.id,
                    "No child-wrappable sandbox active; running trigger unsandboxed"
                );
            }
            (def.interpreter().to_string(), Vec::new())
        }
    };

    let token = new_token();
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&args)
        .env(ENV_TRIGGER_ID, &def.id)
        .env(ENV_TRIGGER_AGENT, &def.agent_id)
        .env(ENV_TRIGGER_ENDPOINT, endpoint.to_string())
        .env(ENV_TRIGGER_TOKEN, &token)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("Failed to spawn trigger '{}'", def.id))?;

    // Write the code to the child's stdin and close it (EOF), so the
    // interpreter reads it as its script. Under a sandbox wrapper the code
    // flows through the wrapper's inherited stdin to the inner interpreter.
    // The async write yields while the interpreter drains the pipe, so a
    // script larger than the pipe buffer cannot deadlock.
    if let Some(mut stdin) = child.stdin.take() {
        let code = def.code.clone();
        stdin
            .write_all(code.as_bytes())
            .await
            .with_context(|| format!("Failed to send trigger '{}' code", def.id))?;
        let _ = stdin.shutdown().await;
    }

    Ok((child, token))
}

/// 256-bit random bearer token, hex-encoded.
fn new_token() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Handle one callback connection: a minimal HTTP/1.1 `POST /fire` with a
/// JSON body `{"token": "...", "context": <any JSON>}`.
async fn handle_fire_connection(
    mut stream: tokio::net::TcpStream,
    tokens: TokenMap,
    fire_tx: mpsc::Sender<TriggerFire>,
) -> Result<()> {
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];

    // Read until end of headers.
    let header_end = loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            anyhow::bail!("connection closed before headers");
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > MAX_REQUEST_BYTES {
            respond(&mut stream, 413, "payload too large").await?;
            anyhow::bail!("headers too large");
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    if !request_line.starts_with("POST /fire") {
        respond(&mut stream, 404, "not found").await?;
        anyhow::bail!("unsupported request: {}", request_line);
    }
    let content_length: usize = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0);
    if content_length == 0 || header_end + content_length > MAX_REQUEST_BYTES {
        respond(&mut stream, 400, "bad request").await?;
        anyhow::bail!("missing or oversized body");
    }

    // Read the remainder of the body.
    while buf.len() < header_end + content_length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            anyhow::bail!("connection closed mid-body");
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = &buf[header_end..header_end + content_length];

    #[derive(serde::Deserialize)]
    struct FireRequest {
        token: String,
        #[serde(default)]
        context: serde_json::Value,
    }
    let req: FireRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(_) => {
            respond(&mut stream, 400, "invalid JSON").await?;
            anyhow::bail!("invalid JSON body");
        }
    };

    let target = tokens.lock().await.get(&req.token).cloned();
    let Some((trigger_id, agent_id)) = target else {
        respond(&mut stream, 401, "unknown token").await?;
        anyhow::bail!("unknown trigger token");
    };

    debug!(trigger = %trigger_id, agent = %agent_id, "Trigger fired");
    if fire_tx
        .send(TriggerFire {
            trigger_id,
            agent_id,
            context: req.context,
        })
        .await
        .is_err()
    {
        // The fire consumer is gone (e.g. gateway shutting down): tell the
        // trigger its event was not accepted rather than falsely ack'ing it.
        return respond(&mut stream, 503, "fire consumer unavailable").await;
    }
    respond(&mut stream, 202, "accepted").await
}

async fn respond(stream: &mut tokio::net::TcpStream, status: u16, text: &str) -> Result<()> {
    let reason = match status {
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "OK",
    };
    let body = format!("{}\n", text);
    let resp = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        body.len(),
        body
    );
    stream.write_all(resp.as_bytes()).await?;
    let _ = stream.shutdown().await;
    Ok(())
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_and_unique() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    /// Full lifecycle: a stored trigger's process is spawned by the manager,
    /// fires through the real endpoint using only its environment variables,
    /// and is killed when the gateway's cancellation token fires.
    #[cfg(unix)]
    #[tokio::test]
    async fn manager_runs_trigger_that_fires_and_stops_on_shutdown() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = TriggerStore::open(tmp.path());
        store.upsert(&rustyclaw_core::triggers::TriggerDef {
            id: "e2e".into(),
            name: "E2E".into(),
            description: None,
            agent_id: "main".into(),
            // Pure-bash fire (no curl dependency): POST over /dev/tcp,
            // then idle like a long-running watcher would.
            code: r#"
host="${RUSTYCLAW_TRIGGER_ENDPOINT%:*}"
port="${RUSTYCLAW_TRIGGER_ENDPOINT#*:}"
body="{\"token\":\"$RUSTYCLAW_TRIGGER_TOKEN\",\"context\":{\"from\":\"e2e\",\"agent\":\"$RUSTYCLAW_TRIGGER_AGENT\"}}"
exec 3<>"/dev/tcp/$host/$port"
printf 'POST /fire HTTP/1.1\r\nHost: x\r\nContent-Length: %s\r\n\r\n%s' "${#body}" "$body" >&3
cat <&3 > /dev/null
sleep 600
"#
            .into(),
            interpreter: Some("bash".into()),
            enabled: true,
            sandboxed: false,
            created_by: None,
            created_at: Some(0),
        })?;

        let (fire_tx, mut fire_rx) = mpsc::channel::<TriggerFire>(4);
        let notify = Arc::new(Notify::new());
        let cancel = CancellationToken::new();
        let mgr = tokio::spawn(run_trigger_manager(
            tmp.path().to_path_buf(),
            tmp.path().join("workspace"),
            fire_tx,
            notify,
            cancel.clone(),
        ));

        // The spawned trigger process should fire within a few seconds.
        let fire = tokio::time::timeout(Duration::from_secs(15), fire_rx.recv())
            .await
            .expect("trigger did not fire in time")
            .expect("fire channel closed");
        assert_eq!(fire.trigger_id, "e2e");
        assert_eq!(fire.agent_id, "main");
        assert_eq!(fire.context["from"], "e2e");
        assert_eq!(fire.context["agent"], "main");

        // Shutdown stops the manager (and kills the sleeping child).
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(10), mgr)
            .await
            .expect("manager did not stop on cancel")??;
        Ok(())
    }

    /// A trigger whose code exits immediately must not be respawned on every
    /// sync tick: it stays tombstoned in `running` until the cooldown lapses.
    #[cfg(unix)]
    #[tokio::test]
    async fn crash_looping_trigger_is_held_back() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = TriggerStore::open(tmp.path());
        store.upsert(&rustyclaw_core::triggers::TriggerDef {
            id: "crasher".into(),
            name: "Crasher".into(),
            description: None,
            agent_id: "main".into(),
            code: "exit 1".into(),
            interpreter: Some("sh".into()),
            enabled: true,
            sandboxed: false,
            created_by: None,
            created_at: Some(0),
        })?;

        let endpoint: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
        let workspace = tmp.path().join("workspace");
        let tokens: TokenMap = Arc::new(Mutex::new(HashMap::new()));
        let mut running: HashMap<String, RunningTrigger> = HashMap::new();

        // First sync spawns the (doomed) process.
        sync(endpoint, &workspace, &store, &mut running, &tokens).await;
        assert!(running.contains_key("crasher"));

        // Give it time to exit, then sync again: it should be reaped and
        // held back as a tombstone (still present, NOT respawned).
        tokio::time::sleep(Duration::from_millis(300)).await;
        sync(endpoint, &workspace, &store, &mut running, &tokens).await;
        let rt = running
            .get("crasher")
            .expect("crashed trigger should be tombstoned, not dropped");
        assert!(
            rt.exited_at.is_some(),
            "tombstone must record the exit and hold back respawn"
        );
        // Its token was withdrawn so a stale process can't fire.
        assert!(tokens.lock().await.is_empty());

        // Simulate the cooldown elapsing (avoids a real 10s wait). The next
        // sync must drop the tombstone and respawn a fresh process — proving
        // the respawn does NOT depend on try_wait succeeding twice.
        running.get_mut("crasher").unwrap().exited_at =
            Instant::now().checked_sub(RESPAWN_COOLDOWN + Duration::from_secs(1));
        sync(endpoint, &workspace, &store, &mut running, &tokens).await;
        let rt = running
            .get("crasher")
            .expect("trigger should be respawned after cooldown");
        assert!(
            rt.exited_at.is_none(),
            "respawned trigger must be a fresh, non-tombstoned entry"
        );

        // Clean up any lingering child.
        for (_, mut rt) in running.drain() {
            let _ = rt.child.start_kill();
        }
        Ok(())
    }

    #[tokio::test]
    async fn fire_endpoint_validates_tokens() -> Result<()> {
        let tokens: TokenMap = Arc::new(Mutex::new(HashMap::new()));
        tokens.lock().await.insert(
            "goodtoken".to_string(),
            ("watcher".to_string(), "main".to_string()),
        );
        let (fire_tx, mut fire_rx) = mpsc::channel::<TriggerFire>(4);

        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let accept_tokens = tokens.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let _ =
                    handle_fire_connection(stream, accept_tokens.clone(), fire_tx.clone()).await;
            }
        });

        let post = |body: String| async move {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            let req = format!(
                "POST /fire HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            s.write_all(req.as_bytes()).await.unwrap();
            let mut resp = String::new();
            let _ = s.read_to_string(&mut resp).await;
            resp
        };

        // Bad token → 401, nothing fired.
        let resp = post(r#"{"token":"wrong","context":{}}"#.to_string()).await;
        assert!(resp.starts_with("HTTP/1.1 401"), "got: {resp}");

        // Good token → 202 and a fire lands with its context.
        let resp = post(r#"{"token":"goodtoken","context":{"path":"/tmp/x"}}"#.to_string()).await;
        assert!(resp.starts_with("HTTP/1.1 202"), "got: {resp}");
        let fire = fire_rx.recv().await.expect("expected a fire");
        assert_eq!(fire.trigger_id, "watcher");
        assert_eq!(fire.agent_id, "main");
        assert_eq!(fire.context["path"], "/tmp/x");

        Ok(())
    }
}
