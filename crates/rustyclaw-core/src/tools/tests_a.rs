//! Tests for the tools module.

#![allow(unused_imports, dead_code)]
use super::*;
use crate::ignore::Ignore;
use std::path::Path;

/// Helper: return the project root as workspace dir for tests.
fn ws() -> &'static Path {
    // In the workspace, CARGO_MANIFEST_DIR is crates/rustyclaw-core.
    // The workspace root is two levels up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

// ── read_file ───────────────────────────────────────────────────

#[test]

fn test_read_file_this_file() {
    let args = json!({ "path": file!(), "start_line": 1, "end_line": 5 });
    let result = exec_read_file(&args, ws());
    assert!(result.is_ok());
    let text = result.unwrap();
    assert!(text.contains("Tests for the tools module"));
}

#[test]
fn test_read_file_missing() {
    let args = json!({ "path": "/nonexistent/file.txt" });
    let result = exec_read_file(&args, ws());
    assert!(result.is_err());
}

#[test]
fn test_read_file_no_path() {
    let args = json!({});
    let result = exec_read_file(&args, ws());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing required parameter")
    );
}

#[test]
fn test_read_file_relative() {
    // Relative path should resolve against workspace_dir.
    let args = json!({ "path": "Cargo.toml", "start_line": 1, "end_line": 3 });
    let result = exec_read_file(&args, ws());
    assert!(result.is_ok());
    let text = result.unwrap();
    assert!(text.contains("workspace"));
}

// ── write_file ──────────────────────────────────────────────────

#[test]
fn test_write_file_and_read_back() {
    let dir = std::env::temp_dir().join("rustyclaw_test_write");
    std::fs::remove_dir_all(&dir).ignore();
    let args = json!({
        "path": "sub/test.txt",
        "content": "hello world"
    });
    let result = exec_write_file(&args, &dir);
    assert!(result.is_ok());
    assert!(result.unwrap().contains("11 bytes"));

    let content = std::fs::read_to_string(dir.join("sub/test.txt")).unwrap();
    assert_eq!(content, "hello world");
    std::fs::remove_dir_all(&dir).ignore();
}

// ── edit_file ───────────────────────────────────────────────────

#[test]
fn test_edit_file_single_match() {
    let dir = std::env::temp_dir().join("rustyclaw_test_edit");
    std::fs::remove_dir_all(&dir).ignore();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.txt"), "aaa\nbbb\nccc\n").unwrap();

    let args = json!({ "path": "f.txt", "old_string": "bbb", "new_string": "BBB" });
    let result = exec_edit_file(&args, &dir);
    assert!(result.is_ok());

    let content = std::fs::read_to_string(dir.join("f.txt")).unwrap();
    assert_eq!(content, "aaa\nBBB\nccc\n");
    std::fs::remove_dir_all(&dir).ignore();
}

#[test]
fn test_edit_file_no_match() {
    let dir = std::env::temp_dir().join("rustyclaw_test_edit_no");
    std::fs::remove_dir_all(&dir).ignore();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.txt"), "aaa\nbbb\n").unwrap();

    let args = json!({ "path": "f.txt", "old_string": "zzz", "new_string": "ZZZ" });
    let result = exec_edit_file(&args, &dir);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
    std::fs::remove_dir_all(&dir).ignore();
}

#[test]
fn test_edit_file_multiple_matches() {
    let dir = std::env::temp_dir().join("rustyclaw_test_edit_multi");
    std::fs::remove_dir_all(&dir).ignore();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.txt"), "aaa\naaa\n").unwrap();

    let args = json!({ "path": "f.txt", "old_string": "aaa", "new_string": "bbb" });
    let result = exec_edit_file(&args, &dir);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("2 times"));
    std::fs::remove_dir_all(&dir).ignore();
}

// ── list_directory ──────────────────────────────────────────────

#[test]
fn test_list_directory() {
    let args = json!({ "path": "crates/rustyclaw-core/src" });
    let result = exec_list_directory(&args, ws());
    assert!(result.is_ok());
    let text = result.unwrap();
    // tools is now a directory
    assert!(text.contains("tools/"));
    assert!(text.contains("lib.rs"));
}

// ── search_files ────────────────────────────────────────────────

#[test]
fn test_search_files_finds_pattern() {
    let args = json!({ "pattern": "exec_read_file", "path": "crates/rustyclaw-core/src", "include": "*.rs" });
    let result = exec_search_files(&args, ws());
    assert!(result.is_ok());
    let text = result.unwrap();
    // The function is now in tools/file.rs
    assert!(text.contains("tools/file.rs") || text.contains("tools\\file.rs"));
}

#[test]
fn test_search_files_no_match() {
    let dir = std::env::temp_dir().join("rustyclaw_test_search_none");
    std::fs::remove_dir_all(&dir).ignore();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "hello world\n").unwrap();

    let args = json!({ "pattern": "XYZZY_NEVER_42" });
    let result = exec_search_files(&args, &dir);
    assert!(result.is_ok());
    assert!(result.unwrap().contains("No matches"));
    std::fs::remove_dir_all(&dir).ignore();
}

// ── find_files ──────────────────────────────────────────────────

#[test]
fn test_find_files_glob() {
    let args = json!({ "pattern": "*.toml" });
    let result = exec_find_files(&args, ws());
    assert!(result.is_ok());
    let text = result.unwrap();
    assert!(text.contains("Cargo.toml"));
}

#[test]
fn test_find_files_keyword_case_insensitive() {
    // "cargo" should match "Cargo.toml" (case-insensitive).
    let args = json!({ "pattern": "cargo" });
    let result = exec_find_files(&args, ws());
    assert!(result.is_ok());
    let text = result.unwrap();
    assert!(text.contains("Cargo.toml"));
}

#[test]
fn test_find_files_multiple_keywords() {
    // Space-separated keywords: match ANY.
    let args = json!({ "pattern": "cargo license" });
    let result = exec_find_files(&args, ws());
    assert!(result.is_ok());
    let text = result.unwrap();
    assert!(text.contains("Cargo.toml"));
    assert!(text.contains("LICENSE"));
}

#[test]
fn test_find_files_keyword_no_match() {
    let dir = std::env::temp_dir().join("rustyclaw_test_find_kw");
    std::fs::remove_dir_all(&dir).ignore();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hello.txt"), "content").unwrap();

    let args = json!({ "pattern": "resume" });
    let result = exec_find_files(&args, &dir);
    assert!(result.is_ok());
    assert!(result.unwrap().contains("No files found"));
    std::fs::remove_dir_all(&dir).ignore();
}

// ── execute_command ─────────────────────────────────────────────

#[test]
fn test_execute_command_echo() {
    let args = json!({ "command": "echo hello" });
    let result = exec_execute_command(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("hello"));
}

#[test]
fn test_execute_command_failure() {
    let args = json!({ "command": "false" });
    let result = exec_execute_command(&args, ws());
    assert!(result.is_ok()); // still returns Ok with exit code
    assert!(result.unwrap().contains("exit code"));
}

/// End-to-end: a foreground exec child registers itself for live status
/// while `execute_command` waits on it, and a `Kill` control action ends
/// the wait and removes it from the registry.
#[cfg(unix)]
#[tokio::test]
async fn test_execute_command_child_is_registered_and_killable() {
    use crate::exec_status::{self, ProcessControlAction};
    use crate::tools::runtime::exec_execute_command_streaming;

    let args = json!({ "command": "sleep 31.7", "timeout_secs": 60, "yieldMs": 50000 });
    let workspace = ws().to_path_buf();
    let handle: tokio::task::JoinHandle<ToolResult> =
        tokio::spawn(async move { exec_execute_command_streaming(&args, &workspace, None).await });

    // Wait for the child to show up in the exec-status registry.
    let mut pid = None;
    for _ in 0..200 {
        if let Some(s) = exec_status::sample_active()
            .into_iter()
            .find(|s| s.command.contains("sleep 31.7"))
        {
            pid = Some(s.pid);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let pid = pid.expect("running exec child should be registered for status");

    exec_status::control(pid, ProcessControlAction::Kill).expect("kill should be accepted");

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
        .await
        .expect("execute_command should return promptly after kill")
        .expect("task join");
    assert!(result.is_ok(), "killed command still returns its output");

    // The guard must have removed the entry when the wait ended.
    assert!(
        !exec_status::sample_active().iter().any(|s| s.pid == pid),
        "registry entry should be gone after the tool returns"
    );
}

/// A command that outlives its yield window is handed over, not run again.
///
/// Backgrounding used to kill the child and re-spawn the command from the
/// top, so anything it had already done happened twice — a build restarted,
/// and a command with a side effect repeated it. The command here appends a
/// line and then lingers: a second copy would leave a second line behind.
#[cfg(unix)]
#[tokio::test]
async fn a_yielding_command_is_not_run_a_second_time() {
    use crate::tools::runtime::exec_execute_command_streaming;

    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("runs");
    let command = format!("echo run >> {}; sleep 30", marker.display());

    let args = json!({ "command": command, "timeout_secs": 60, "yieldMs": 300 });
    let result = exec_execute_command_streaming(&args, ws(), None)
        .await
        .expect("yielding to the background is not an error");
    assert!(
        result.contains("sessionId"),
        "a yielded command reports its session: {result}"
    );

    // Long enough that a re-spawned copy would have written its own line.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let runs = std::fs::read_to_string(&marker).unwrap_or_default();
    assert_eq!(
        runs.lines().count(),
        1,
        "the command should have run once, not been restarted: {runs:?}"
    );
}

/// Output produced before the hand-off is still there afterwards.
///
/// Killing and re-spawning discarded it, so a command that had already
/// printed something came back from backgrounding with nothing to show.
///
/// The line has to be one a second run could not reproduce, or the test
/// passes on the re-spawned copy's identical output and guards nothing:
/// the command counts its own runs, so the first prints `run-1` and any
/// restart prints `run-2`.
#[cfg(unix)]
#[tokio::test]
async fn output_from_before_the_handoff_survives_it() {
    use crate::tools::helpers::process_manager;
    use crate::tools::runtime::exec_execute_command_streaming;

    let dir = tempfile::tempdir().expect("tempdir");
    let counter = dir.path().join("n");
    let command = format!(
        "n=$(cat {c} 2>/dev/null || echo 0); n=$((n+1)); printf %s \"$n\" > {c}; echo run-$n; sleep 30",
        c = counter.display()
    );
    let args = json!({
        "command": command,
        "timeout_secs": 60,
        "yieldMs": 400,
    });
    let result = exec_execute_command_streaming(&args, ws(), None)
        .await
        .expect("yielding to the background is not an error");
    let session_id = serde_json::from_str::<serde_json::Value>(&result)
        .ok()
        .and_then(|v| v["sessionId"].as_str().map(str::to_owned))
        .expect("a yielded command reports its session");

    // The readers keep running, so the line lands whether it was written
    // before or after the hand-off; poll until it shows up.
    let mut seen = String::new();
    for _ in 0..100 {
        {
            let mgr = process_manager();
            let mut mgr = mgr.lock().expect("process manager lock");
            if let Some(session) = mgr.get_mut(&session_id) {
                session.try_read_output();
                seen = session.full_output().to_string();
            }
        }
        if seen.contains("run-") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        seen.contains("run-1"),
        "output from before the hand-off should survive it, got: {seen:?}"
    );
    assert!(
        !seen.contains("run-2"),
        "the session should be watching the original run, not a restart: {seen:?}"
    );

    let mgr = process_manager();
    let mut mgr = mgr.lock().expect("process manager lock");
    if let Some(session) = mgr.get_mut(&session_id) {
        session.kill().ignore();
    }
}

/// An adopted child's exit is noticed, and it is *that* child's exit.
///
/// The exit code counts runs for the same reason the output test does: a
/// re-spawned copy exiting with the same code would satisfy a fixed one,
/// so the first run exits 1 and a restart would exit 2.
#[cfg(unix)]
#[tokio::test]
async fn an_adopted_child_reports_its_own_exit() {
    use crate::process_manager::SessionStatus;
    use crate::tools::helpers::process_manager;
    use crate::tools::runtime::exec_execute_command_streaming;

    let dir = tempfile::tempdir().expect("tempdir");
    let counter = dir.path().join("n");
    // Yields first, then exits on its own.
    let command = format!(
        "n=$(cat {c} 2>/dev/null || echo 0); n=$((n+1)); printf %s \"$n\" > {c}; sleep 0.6; exit $n",
        c = counter.display()
    );
    let args = json!({
        "command": command,
        "timeout_secs": 60,
        "yieldMs": 200,
    });
    let result = exec_execute_command_streaming(&args, ws(), None)
        .await
        .expect("yielding to the background is not an error");
    let session_id = serde_json::from_str::<serde_json::Value>(&result)
        .ok()
        .and_then(|v| v["sessionId"].as_str().map(str::to_owned))
        .expect("a yielded command reports its session");

    // 15s, where this used to allow 4s. The child sleeps 0.6s, so the old
    // budget looked generous — but a loaded CI runner starves this poll loop
    // and it timed out in three separate runs, always with `None` rather than
    // a wrong code. Waiting longer takes nothing away from what the test
    // checks: the claim is about *which* exit code arrives, and a re-spawned
    // copy would still report 2 however long we wait for it.
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(20);
    const POLL_ATTEMPTS: usize = 750;

    let mut status = None;
    for _ in 0..POLL_ATTEMPTS {
        {
            let mgr = process_manager();
            let mut mgr = mgr.lock().expect("process manager lock");
            if let Some(session) = mgr.get_mut(&session_id)
                && session.check_exit()
            {
                status = Some(session.status.clone());
            }
        }
        if status.is_some() {
            break;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    // Separated from the assertion below because the two failures mean
    // entirely different things: no exit at all is this loop being too
    // impatient, while a wrong code is the bug the test exists to catch.
    // Folding them together produced a `None != Some(Exited(1))` that needed
    // a trip to the CI log to tell one from the other.
    let status = status.unwrap_or_else(|| {
        panic!(
            "no exit reported within {:?} — the child never finished, which is \
             a stalled or starved poll rather than a re-spawn",
            POLL_INTERVAL * POLL_ATTEMPTS as u32
        )
    });
    assert_eq!(
        status,
        SessionStatus::Exited(1),
        "the session should report the original child's exit, not a restart's"
    );
}

/// Named keys reach a command that has moved to the background.
///
/// An adopted session has no `child` — its stdin goes through the
/// supervisor — so every stdin entry point has to know that. `write_stdin`
/// did and `send_keys` did not, which turned "press Enter" on a
/// backgrounded interactive command into "Process has exited" while it was
/// plainly still running.
#[cfg(unix)]
#[tokio::test]
async fn keys_reach_a_command_after_it_moves_to_the_background() {
    use crate::tools::helpers::process_manager;
    use crate::tools::runtime::exec_execute_command_streaming;

    // Echoes whatever it is given, so anything that arrives comes back.
    let args = json!({ "command": "cat", "timeout_secs": 60, "yieldMs": 300 });
    let result = exec_execute_command_streaming(&args, ws(), None)
        .await
        .expect("yielding to the background is not an error");
    let session_id = serde_json::from_str::<serde_json::Value>(&result)
        .ok()
        .and_then(|v| v["sessionId"].as_str().map(str::to_owned))
        .expect("a yielded command reports its session");

    {
        let mgr = process_manager();
        let mut mgr = mgr.lock().expect("process manager lock");
        let session = mgr.get_mut(&session_id).expect("the adopted session");
        session
            .write_stdin("typed")
            .expect("typed text should reach a backgrounded command");
        session
            .send_keys("Enter")
            .expect("named keys should reach a backgrounded command too");
    }

    // `cat` only emits the line once the Enter lands, so seeing it back
    // proves the keystroke arrived rather than just being accepted.
    let mut seen = String::new();
    for _ in 0..100 {
        {
            let mgr = process_manager();
            let mut mgr = mgr.lock().expect("process manager lock");
            if let Some(session) = mgr.get_mut(&session_id) {
                session.try_read_output();
                seen = session.full_output().to_string();
            }
        }
        if seen.contains("typed") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        seen.contains("typed"),
        "the keystroke should have reached the command, got: {seen:?}"
    );

    let mgr = process_manager();
    let mut mgr = mgr.lock().expect("process manager lock");
    if let Some(session) = mgr.get_mut(&session_id) {
        session.kill().ignore();
    }
}

/// An adopted session collects a large burst and reports the right exit.
///
/// This covers the adoption path under load — 400KB through the pipes,
/// across many reads, with the exit code arriving intact. It does **not**
/// prove the ordering fix that accompanies it: the supervisor now waits
/// for pipe EOF before publishing the status, so a session cannot report
/// finishing while the readers still hold its closing lines, but that
/// window is a few microseconds of scheduler latency and this test passes
/// with the wait removed. Any polling loop tight enough to land inside the
/// window also gives the readers the scheduling they need to close it.
/// Left here for what it does cover, and deliberately not described as a
/// guard for the race.
#[cfg(unix)]
#[tokio::test]
async fn a_backgrounded_command_keeps_its_closing_lines() {
    use crate::process_manager::SessionStatus;
    use crate::tools::helpers::process_manager;
    use crate::tools::runtime::exec_execute_command_streaming;

    // Yields, then prints a burst and exits immediately after — the window
    // in which the readers are still behind the supervisor.
    // A big burst, so the readers need many reads to keep up and cannot
    // finish inside the instant the supervisor takes to record the exit.
    // A handful of lines is drained faster than the status is observed,
    // which made an earlier version of this test pass on the bug.
    //
    // The timeout is deliberately far beyond the command's real runtime
    // (~1s). This test's claim is about which status and which closing lines
    // arrive when the session reports finishing — not about wall-clock speed —
    // and a loaded CI runner starved the 60s budget once (Some(TimedOut)
    // instead of Some(Exited(3))), which is a slow runner, not the bug. A
    // longer timeout takes nothing away from the check.
    let args = json!({
        "command": "sleep 0.4; awk 'BEGIN{for(i=0;i<20000;i++) print \"closing-line-\" i}'; echo END-MARKER; exit 3",
        "timeout_secs": 300,
        "yieldMs": 200,
    });
    let result = exec_execute_command_streaming(&args, ws(), None)
        .await
        .expect("yielding to the background is not an error");
    let session_id = serde_json::from_str::<serde_json::Value>(&result)
        .ok()
        .and_then(|v| v["sessionId"].as_str().map(str::to_owned))
        .expect("a yielded command reports its session");

    // Poll exactly as the process tool does, and stop at the first poll
    // that reports the session finished — the point after which nothing
    // would collect any more output.
    // Polled as tightly as possible: the window this guards is the instant
    // between the exit being recorded and the readers catching up, and
    // sleeping between polls hands them the time to close it.
    //
    // Bounded by TIME, not iterations: an iteration cap is a race against
    // the child — on a loaded CI runner a hot yield loop spins through any
    // count before a busy child exits, and the test fails without testing
    // anything. The child takes ~1s; a generous wall clock changes nothing
    // when it passes and only matters when it would have lied.
    let mut seen = String::new();
    let mut status = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        {
            let mgr = process_manager();
            let mut mgr = mgr.lock().expect("process manager lock");
            if let Some(session) = mgr.get_mut(&session_id) {
                session.try_read_output();
                let done = session.check_exit();
                if done {
                    seen = session.full_output().to_string();
                    status = Some(session.status.clone());
                }
            }
        }
        if status.is_some() {
            break;
        }
        tokio::task::yield_now().await;
    }

    assert_eq!(
        status,
        Some(SessionStatus::Exited(3)),
        "the command should have been seen to finish"
    );
    assert!(
        seen.contains("END-MARKER"),
        "the last line printed before exiting should be present by the time \
         the session reports finishing; got {} bytes ending {:?}",
        seen.len(),
        &seen[seen.len().saturating_sub(80)..]
    );
}

/// Time spent paused is not billed to the command once it backgrounds.
///
/// The foreground loop pushes both deadlines out while the user has a
/// process stopped, so a paused command cannot time out underneath them.
/// The session measures its timeout from the command's real start, so
/// handing over the timeout the *arguments* asked for rather than the one
/// the loop had arrived at gave that grace straight back — far enough here
/// that the very first poll would kill it.
#[cfg(unix)]
#[tokio::test]
async fn a_paused_command_keeps_its_grace_when_it_backgrounds() {
    use crate::exec_status::{self, ProcessControlAction};
    use crate::process_manager::SessionStatus;
    use crate::tools::helpers::process_manager;
    use crate::tools::runtime::exec_execute_command_streaming;

    // Paused for longer than the raw timeout, so elapsed-since-start is
    // past it by the time the yield fires, while the adjusted budget is
    // not. Both deadlines freeze while stopped, so the yield lands after
    // the pause rather than during it.
    let args = json!({ "command": "sleep 30", "timeout_secs": 3, "yieldMs": 200 });
    let workspace = ws().to_path_buf();
    let handle: tokio::task::JoinHandle<ToolResult> =
        tokio::spawn(async move { exec_execute_command_streaming(&args, &workspace, None).await });

    let mut pid = None;
    for _ in 0..200 {
        if let Some(s) = exec_status::sample_active()
            .into_iter()
            .find(|s| s.command == "sleep 30" && s.alive)
        {
            pid = Some(s.pid);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let pid = pid.expect("the running child should be registered");

    exec_status::control(pid, ProcessControlAction::Pause).expect("pause");
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    exec_status::control(pid, ProcessControlAction::Resume).expect("resume");

    let result = tokio::time::timeout(std::time::Duration::from_secs(20), handle)
        .await
        .expect("the command should yield rather than hang")
        .expect("task join")
        .expect("yielding to the background is not an error");
    let session_id = serde_json::from_str::<serde_json::Value>(&result)
        .ok()
        .and_then(|v| v["sessionId"].as_str().map(str::to_owned))
        .expect("a yielded command reports its session");

    let mgr = process_manager();
    let mut mgr = mgr.lock().expect("process manager lock");
    let session = mgr.get_mut(&session_id).expect("the adopted session");
    let exited = session.check_exit();
    let status = session.status.clone();
    session.kill().ignore();

    assert!(
        !exited && status == SessionStatus::Running,
        "a command paused for longer than its raw timeout should not be \
         timed out on its first poll, got: {status:?}"
    );
}

// ── execute_tool dispatch ───────────────────────────────────────

#[tokio::test]
async fn test_execute_tool_dispatch() {
    let args = json!({ "path": file!() });
    let result = execute_tool("read_file", &args, ws()).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_execute_tool_unknown() {
    let result = execute_tool("no_such_tool", &json!({}), ws()).await;
    assert!(result.is_err());
}

// ── Provider format tests ───────────────────────────────────────

#[test]
fn test_openai_format() {
    let tools = tools_openai();
    assert!(
        tools.len() >= 60,
        "Expected at least 60 tools, got {}",
        tools.len()
    );
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["function"]["name"], "read_file");
    assert!(tools[0]["function"]["parameters"]["properties"]["path"].is_object());
}

#[test]
fn test_anthropic_format() {
    let tools = tools_anthropic();
    assert!(
        tools.len() >= 60,
        "Expected at least 60 tools, got {}",
        tools.len()
    );
    assert_eq!(tools[0]["name"], "read_file");
    assert!(tools[0]["input_schema"]["properties"]["path"].is_object());
}

#[test]
fn test_google_format() {
    let tools = tools_google();
    assert!(
        tools.len() >= 60,
        "Expected at least 60 tools, got {}",
        tools.len()
    );
    assert_eq!(tools[0]["name"], "read_file");
}

// ── resolve_path helper ─────────────────────────────────────────

#[test]
fn test_resolve_path_absolute() {
    let result = helpers::resolve_path(Path::new("/workspace"), "/absolute/path.txt");
    assert_eq!(result, std::path::PathBuf::from("/absolute/path.txt"));
}

#[test]
fn test_resolve_path_relative() {
    let result = helpers::resolve_path(Path::new("/workspace"), "relative/path.txt");
    assert_eq!(
        result,
        std::path::PathBuf::from("/workspace/relative/path.txt")
    );
}

// ── web_fetch ───────────────────────────────────────────────────

#[test]
fn test_web_fetch_missing_url() {
    let args = json!({});
    let result = exec_web_fetch(&args, ws());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing required parameter")
    );
}

#[test]
fn test_web_fetch_invalid_url() {
    let args = json!({ "url": "not-a-url" });
    let result = exec_web_fetch(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("http"));
}

#[test]
fn test_web_fetch_blocks_localhost_ssrf() {
    // SSRF: requests to loopback must be rejected before any network I/O.
    let args = json!({ "url": "http://127.0.0.1/" });
    let result = exec_web_fetch(&args, ws());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("blocked") || err.to_string().contains("Security"),
        "expected SSRF rejection, got: {err}"
    );
}

#[test]
fn test_web_fetch_blocks_private_ip_ssrf() {
    // SSRF: RFC-1918 private range must be rejected.
    let args = json!({ "url": "http://10.0.0.1/" });
    let result = exec_web_fetch(&args, ws());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("blocked") || err.to_string().contains("Security"),
        "expected SSRF rejection, got: {err}"
    );
}

#[test]
fn test_web_fetch_blocks_cloud_metadata_ssrf() {
    // SSRF: the cloud metadata endpoint is the classic exfiltration target.
    let args = json!({ "url": "http://169.254.169.254/latest/meta-data/" });
    let result = exec_web_fetch(&args, ws());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("blocked") || err.to_string().contains("Security"),
        "expected SSRF rejection, got: {err}"
    );
}

#[test]
fn test_web_fetch_params_defined() {
    let params = web_fetch_params();

    // Named rather than counted. The count assertion this replaces could not
    // fail for a missing parameter — `to_file` was described to the model in
    // the tool's prose, read by `exec_web_fetch`, and never declared here, and
    // a `len() == 6` that nobody had reason to touch stayed green throughout.
    let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "url",
            "extract_mode",
            "max_chars",
            "use_cookies",
            "authorization",
            "headers",
            "to_file",
        ]
    );

    assert!(params.iter().any(|p| p.name == "url" && p.required));
    for optional in [
        "extract_mode",
        "max_chars",
        "use_cookies",
        "authorization",
        "headers",
        "to_file",
    ] {
        assert!(
            params.iter().any(|p| p.name == optional && !p.required),
            "{optional} should be declared optional"
        );
    }
}

/// Every argument `exec_web_fetch` reads has to be declared in the schema the
/// providers receive: one that is only described in prose is an argument a
/// strict-validating provider will strip or reject, so the feature behind it
/// is unreachable no matter how well the runtime handles it.
#[test]
fn test_web_fetch_reads_no_undeclared_arguments() {
    let declared: std::collections::HashSet<String> =
        web_fetch_params().into_iter().map(|p| p.name).collect();

    let src = include_str!("web.rs");
    let mut read: Vec<String> = Vec::new();
    for (i, m) in src.match_indices("args.get(\"") {
        let rest = &src[i + m.len()..];
        if let Some(end) = rest.find('"') {
            read.push(rest[..end].to_string());
        }
    }
    assert!(
        !read.is_empty(),
        "found no argument reads at all — the scan is broken, not the schema"
    );

    // `web.rs` holds web_search too, so only complain about names neither
    // tool declares.
    let search: std::collections::HashSet<String> =
        web_search_params().into_iter().map(|p| p.name).collect();
    let undeclared: Vec<&String> = read
        .iter()
        .filter(|n| !declared.contains(*n) && !search.contains(*n))
        .collect();
    assert!(
        undeclared.is_empty(),
        "read at runtime but never declared in any tool schema: {undeclared:?}"
    );
}

// ── web_search ──────────────────────────────────────────────────

#[test]
fn test_web_search_missing_query() {
    let args = json!({});
    let result = exec_web_search(&args, ws());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing required parameter")
    );
}

#[test]
fn test_web_search_no_api_key() {
    // Clear any existing key for the test
    // SAFETY: This test is single-threaded and no other thread reads BRAVE_API_KEY.
    unsafe { std::env::remove_var("BRAVE_API_KEY") };
    let args = json!({ "query": "test" });
    let result = exec_web_search(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("BRAVE_API_KEY"));
}

#[test]
fn test_web_search_params_defined() {
    let params = web_search_params();
    assert_eq!(params.len(), 5);
    assert!(params.iter().any(|p| p.name == "query" && p.required));
    assert!(params.iter().any(|p| p.name == "count" && !p.required));
    assert!(params.iter().any(|p| p.name == "country" && !p.required));
    assert!(
        params
            .iter()
            .any(|p| p.name == "search_lang" && !p.required)
    );
    assert!(params.iter().any(|p| p.name == "freshness" && !p.required));
}

// ── process ─────────────────────────────────────────────────────

#[test]
fn test_process_missing_action() {
    let args = json!({});
    let result = exec_process(&args, ws());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing required parameter")
    );
}

#[test]
fn test_process_invalid_action() {
    let args = json!({ "action": "invalid" });
    let result = exec_process(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Unknown action"));
}

#[test]
fn test_process_list_empty() {
    let args = json!({ "action": "list" });
    let result = exec_process(&args, ws());
    assert!(result.is_ok());
    // May have sessions from other tests, so just check it doesn't error
}

#[test]
fn test_process_params_defined() {
    let params = process_params();
    assert_eq!(params.len(), 6);
    assert!(params.iter().any(|p| p.name == "action" && p.required));
    assert!(params.iter().any(|p| p.name == "sessionId" && !p.required));
    assert!(params.iter().any(|p| p.name == "data" && !p.required));
    assert!(params.iter().any(|p| p.name == "keys" && !p.required));
    assert!(params.iter().any(|p| p.name == "offset" && !p.required));
    assert!(params.iter().any(|p| p.name == "limit" && !p.required));
}

#[test]
fn test_execute_command_params_with_background() {
    let params = execute_command_params();
    assert_eq!(params.len(), 5);
    assert!(params.iter().any(|p| p.name == "command" && p.required));
    assert!(params.iter().any(|p| p.name == "background" && !p.required));
    assert!(params.iter().any(|p| p.name == "yieldMs" && !p.required));
}

// ── memory_search ───────────────────────────────────────────────

#[test]
fn test_memory_search_params_defined() {
    let params = memory_search_params();
    assert_eq!(params.len(), 5);
    assert!(params.iter().any(|p| p.name == "query" && p.required));
    assert!(params.iter().any(|p| p.name == "maxResults" && !p.required));
    assert!(params.iter().any(|p| p.name == "minScore" && !p.required));
    assert!(
        params
            .iter()
            .any(|p| p.name == "recencyBoost" && !p.required)
    );
    assert!(
        params
            .iter()
            .any(|p| p.name == "halfLifeDays" && !p.required)
    );
}

#[cfg(feature = "semantic-memory")]
#[test]
fn test_memory_search_missing_query() {
    let args = json!({});
    let result = exec_memory_search(&args, ws());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing required parameter")
    );
}

// ── memory_get ──────────────────────────────────────────────────

#[test]
fn test_memory_get_params_defined() {
    let params = memory_get_params();
    assert_eq!(params.len(), 3);
    assert!(params.iter().any(|p| p.name == "path" && p.required));
    assert!(params.iter().any(|p| p.name == "from" && !p.required));
    assert!(params.iter().any(|p| p.name == "lines" && !p.required));
}

#[test]
fn test_memory_get_missing_path() {
    let args = json!({});
    let result = exec_memory_get(&args, ws());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing required parameter")
    );
}

#[test]
fn test_memory_get_invalid_path() {
    let args = json!({ "path": "../etc/passwd" });
    let result = exec_memory_get(&args, ws());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("not a valid memory file")
    );
}

// ── cron ────────────────────────────────────────────────────────

#[test]
fn test_cron_params_defined() {
    let params = cron_params();
    assert_eq!(params.len(), 5);
    assert!(params.iter().any(|p| p.name == "action" && p.required));
    assert!(params.iter().any(|p| p.name == "jobId" && !p.required));
}

#[test]
fn test_cron_missing_action() {
    let args = json!({});
    let result = exec_cron(&args, ws());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing required parameter")
    );
}

#[test]
fn test_cron_invalid_action() {
    // `exec_cron` opens the central cron store under the gateway settings
    // dir, which nothing sets in this module — whether the error below is
    // even reachable depends on which other test module happened to publish
    // it first (agent/swarm/trigger tests do, and a fast sync test can run
    // before them). Pin the context so this test is the same everywhere,
    // exactly as the agent/swarm tests do.
    let _guard = crate::runtime_ctx::TEST_CTX_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    crate::runtime_ctx::set_agent_registry_info(tmp.path(), "RustyClaw");

    let args = json!({ "action": "invalid" });
    let result = exec_cron(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Unknown action"));
}

// ── sessions_list ───────────────────────────────────────────────

#[test]
fn test_sessions_list_params_defined() {
    let params = sessions_list_params();
    assert_eq!(params.len(), 4);
    assert!(params.iter().all(|p| !p.required));
}

// ── sessions_spawn ──────────────────────────────────────────────

#[test]
fn test_sessions_spawn_params_defined() {
    let params = sessions_spawn_params();
    assert_eq!(params.len(), 5);
    assert!(params.iter().any(|p| p.name == "task" && p.required));
}

#[test]
fn test_sessions_spawn_missing_task() {
    let args = json!({});
    let result = exec_sessions_spawn(&args, ws());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing required parameter")
    );
}

/// Hold while touching the process-wide spawn runner, and leave it absent
/// afterwards so the tests that expect no gateway still see none.
fn spawn_runner_guard() -> std::sync::MutexGuard<'static, ()> {
    let guard = crate::sessions::SPAWN_RUNNER_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    crate::sessions::clear_spawn_runner();
    guard
}

#[test]
fn test_sessions_spawn_without_a_runner_refuses() {
    let _guard = spawn_runner_guard();
    // No gateway is hosting these tools in a unit test, so there is nothing
    // that can run an agent turn. The tool must say so rather than file a
    // session record for work that will never happen — the whole defect in
    // #114.
    let result = exec_sessions_spawn(&json!({"task": "do a thing"}), ws());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no gateway"), "{err}");
}

#[test]
fn test_sessions_spawn_leaves_no_record_when_it_refuses() {
    let _guard = spawn_runner_guard();
    // Identified by task text rather than by counting: the session manager is
    // process-wide, so a count would race with any other test spawning one.
    const TASK: &str = "sessions_spawn refusal leaves no record";
    exec_sessions_spawn(&json!({"task": TASK}), ws())
        .expect_err("no runner is installed, so this must refuse");

    let mgr = crate::sessions::session_manager().lock().unwrap();
    assert!(
        !mgr.list(None, false, usize::MAX)
            .iter()
            .any(|s| s.task.as_deref() == Some(TASK)),
        "a refused spawn must not leave a session behind"
    );
}

#[test]
fn test_sessions_spawn_hands_the_run_to_the_installed_runner() {
    use crate::sessions::{SpawnRequest, SpawnRunner};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Recorder(Mutex<Vec<SpawnRequest>>);
    impl SpawnRunner for Recorder {
        fn start(&self, req: SpawnRequest) -> Result<(), String> {
            self.0.lock().unwrap().push(req);
            Ok(())
        }
    }

    let _guard = spawn_runner_guard();
    let recorder = Arc::new(Recorder::default());
    crate::sessions::set_spawn_runner(recorder.clone());

    let out = exec_sessions_spawn(
        &json!({"task": "summarize the log", "runTimeoutSeconds": 30, "model": "haiku"}),
        ws(),
    )
    .unwrap();
    assert!(out.contains("\"running\""), "{out}");

    let started = recorder.0.lock().unwrap();
    let req = started
        .iter()
        .find(|r| r.task == "summarize the log")
        .expect("the runner must be handed the request");
    assert_eq!(req.agent_id, "main");
    assert_eq!(req.model.as_deref(), Some("haiku"));
    // The schema advertises runTimeoutSeconds, so it has to reach the run.
    assert_eq!(req.timeout_secs, Some(30));

    // The record the parent will poll exists and carries the task.
    let mgr = crate::sessions::session_manager().lock().unwrap();
    let session = mgr.get(&req.session_key).expect("session record");
    assert_eq!(session.status, crate::sessions::SessionStatus::Active);
    assert!(session.messages.iter().any(|m| m.role == "user"));
    drop(mgr);
    drop(started);
    crate::sessions::clear_spawn_runner();
}

// ── sessions_kill ───────────────────────────────────────────────

#[test]
fn test_sessions_kill_params_defined() {
    let params = sessions_kill_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().all(|p| !p.required));
}

#[test]
fn test_sessions_kill_needs_a_target() {
    let err = exec_sessions_kill(&json!({}), ws())
        .unwrap_err()
        .to_string();
    assert!(err.contains("sessionKey or label"), "{err}");
}

#[test]
fn test_sessions_kill_unknown_session() {
    let err = exec_sessions_kill(&json!({"sessionKey": "agent:main:subagent:nope"}), ws())
        .unwrap_err()
        .to_string();
    assert!(err.contains("No session found"), "{err}");
}

#[test]
fn test_sessions_kill_marks_the_record_stopped() {
    let key = {
        let mut mgr = crate::sessions::session_manager().lock().unwrap();
        mgr.spawn_subagent("main", "long job", None, None)
    };
    let out = exec_sessions_kill(&json!({"sessionKey": key.clone()}), ws()).unwrap();
    assert!(out.contains("stopped"), "{out}");

    let mgr = crate::sessions::session_manager().lock().unwrap();
    assert_eq!(
        mgr.get(&key).unwrap().status,
        crate::sessions::SessionStatus::Stopped,
        "a stopped run must not report itself Completed — the parent would \
         read that as the work being done"
    );
}

#[test]
fn test_sessions_kill_refuses_another_callers_session() {
    let key = {
        let mut mgr = crate::sessions::session_manager().lock().unwrap();
        mgr.spawn_subagent("main", "someone else's job", None, Some("thread:1".into()))
    };
    // Unidentified here, so the caller is not `thread:1`. The refusal is
    // phrased as "not found" so one conversation cannot probe another's keys.
    let err = exec_sessions_kill(&json!({"sessionKey": key.clone()}), ws())
        .unwrap_err()
        .to_string();
    assert!(err.contains("No session found"), "{err}");

    let mgr = crate::sessions::session_manager().lock().unwrap();
    assert_eq!(
        mgr.get(&key).unwrap().status,
        crate::sessions::SessionStatus::Active
    );
}

#[test]
fn test_sessions_kill_is_idempotent() {
    let key = {
        let mut mgr = crate::sessions::session_manager().lock().unwrap();
        mgr.spawn_subagent("main", "short job", None, None)
    };
    exec_sessions_kill(&json!({"sessionKey": key.clone()}), ws()).unwrap();
    let out = exec_sessions_kill(&json!({"sessionKey": key}), ws()).unwrap();
    assert!(out.contains("already"), "{out}");
}

// ── sessions_send ───────────────────────────────────────────────

#[test]
fn test_sessions_send_params_defined() {
    let params = sessions_send_params();
    assert_eq!(params.len(), 4);
    assert!(params.iter().any(|p| p.name == "message" && p.required));
}

#[test]
fn test_sessions_send_missing_message() {
    let args = json!({});
    let result = exec_sessions_send(&args, ws());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing required parameter")
    );
}

// ── sessions_history ────────────────────────────────────────────

#[test]
fn test_sessions_history_params_defined() {
    let params = sessions_history_params();
    assert_eq!(params.len(), 3);
    assert!(params.iter().any(|p| p.name == "sessionKey" && p.required));
}

// ── session_status ──────────────────────────────────────────────

#[test]
fn test_session_status_params_defined() {
    let params = session_status_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().all(|p| !p.required));
}

#[test]
fn test_session_status_general() {
    let args = json!({});
    let result = exec_session_status(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("Session Status"));
}

// ── agents_list ─────────────────────────────────────────────────

#[test]
fn test_agents_list_params_defined() {
    let params = agents_list_params();
    assert_eq!(params.len(), 0);
}

#[test]
fn test_agents_list_returns_main() {
    let args = json!({});
    let result = exec_agents_list(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("main"));
}

// ── apply_patch ─────────────────────────────────────────────────

#[test]
fn test_apply_patch_params_defined() {
    let params = apply_patch_params();
    assert_eq!(params.len(), 3);
    assert!(params.iter().any(|p| p.name == "patch" && p.required));
    assert!(params.iter().any(|p| p.name == "dry_run" && !p.required));
}

#[test]
fn test_apply_patch_missing_patch() {
    let args = json!({});
    let result = exec_apply_patch(&args, ws());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing required parameter")
    );
}

#[test]
fn test_parse_unified_diff() {
    let patch_str = r#"--- a/test.txt
+++ b/test.txt
@@ -1,3 +1,4 @@
 line1
+new line
 line2
 line3
"#;
    let hunks = patch::parse_unified_diff(patch_str).unwrap();
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].file_path, "test.txt");
    assert_eq!(hunks[0].old_start, 1);
    assert_eq!(hunks[0].old_count, 3);
}

// ── secrets tools ───────────────────────────────────────────────

/// The parameter list is one step removed from what providers actually see:
/// `definitions.rs` declares `parameters: vec![]` and the real list is looked
/// up by name in `resolve_params`, whose fallback arm is a silent `vec![]`. So
/// this checks the delivered payload rather than the list feeding it.
#[test]
fn test_web_fetch_schema_reaches_providers_with_to_file() {
    for (provider, tools) in [
        ("anthropic", crate::tools::schema::tools_anthropic()),
        ("openai", crate::tools::schema::tools_openai()),
        ("google", crate::tools::schema::tools_google()),
    ] {
        let tool = tools
            .iter()
            .find(|t| {
                t.get("name").and_then(|n| n.as_str()) == Some("web_fetch")
                    || t.get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        == Some("web_fetch")
            })
            .unwrap_or_else(|| panic!("{provider}: web_fetch missing from the tool list"));

        let json = serde_json::to_string(tool).unwrap();
        assert!(
            json.contains("to_file"),
            "{provider}: web_fetch schema does not mention to_file, so the model \
             cannot legally ask for a download"
        );
    }
}

/// `reqwest`'s `ClientBuilder::timeout` is a *total* deadline — the docs say it
/// runs "from when the request starts connecting until the response body has
/// finished" — so applying the read path's 30s to a download kills every
/// transfer this mode exists for, part-written, at the 30s mark.
#[test]
fn test_download_gets_no_total_deadline() {
    use crate::tools::web::Deadline;

    assert!(
        matches!(Deadline::for_fetch(true), Deadline::Streaming { .. }),
        "a download must not be given a total deadline"
    );
    assert!(
        matches!(Deadline::for_fetch(false), Deadline::Total(_)),
        "an ordinary read should keep its total deadline"
    );
}

/// A download is the one write the model can start that nothing else bounds.
///
/// `write_file` is limited by what the model can emit and a non-download
/// `web_fetch` by `max_chars`, but a transfer is detached from the turn on
/// purpose — no turn ends it — and its streaming deadline bounds a stall
/// rather than a body that simply keeps arriving. So the cap is not parity
/// with the other write paths; it is the thing they have that this lacks.
#[test]
fn test_download_size_is_capped() {
    use crate::tools::web::{MAX_DOWNLOAD_BYTES, exceeds_download_cap, too_large};

    // An ordinary large download passes: the cap is a backstop against an
    // unbounded stream, not a policy about how big a file may be fetched.
    assert!(!exceeds_download_cap(4 * 1024 * 1024 * 1024));
    // The boundary itself is allowed; one byte past it is not. Both checks
    // share this predicate, so they cannot disagree about which is which.
    assert!(!exceeds_download_cap(MAX_DOWNLOAD_BYTES));
    assert!(exceeds_download_cap(MAX_DOWNLOAD_BYTES + 1));

    // The reason has to name both numbers: without them the agent cannot tell
    // a cap it could work under from a transport failure it should retry.
    let msg = too_large(MAX_DOWNLOAD_BYTES + 1);
    assert!(
        msg.contains("16.0 GB"),
        "the limit itself should be stated: {msg}"
    );
    assert!(
        msg.contains("maximum download size"),
        "the reason should say it was a size limit: {msg}"
    );
}

/// Deleting an agent takes its conversations with it, and an agent later
/// created under the same id starts clean.
///
/// The gateway's frame handler is not the only way an agent disappears:
/// `agents_delete` runs in this process, and so does swarm teardown. All of
/// them reach `AgentRegistry::delete`, which is why the store is forgotten
/// there rather than at each caller.
#[test]
fn deleting_an_agent_takes_its_threads_with_it() {
    use crate::agents::AgentRegistry;
    use crate::threads::ThreadStore;

    let root = std::env::temp_dir().join(format!("rustyclaw-agent-delete-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ignore();
    std::fs::create_dir_all(&root).unwrap();

    let registry = AgentRegistry::new(&root, "Main");
    registry
        .create(Some("doomed"), "Doomed", None, None, None)
        .expect("create");
    let threads_path = registry.agent_dir("doomed").join("sessions/threads.json");
    std::fs::create_dir_all(threads_path.parent().unwrap()).unwrap();

    // A conversation that really reaches disk.
    {
        let manager = crate::threads::manager_for(&threads_path);
        let mut tm = manager.blocking_lock();
        tm.create_chat("secret plans");
        ThreadStore::at_legacy_path(&threads_path)
            .persist(&mut tm)
            .expect("persist");
    }
    // Nothing holds it now, so the delete is allowed to proceed.

    registry.delete("doomed").expect("delete");

    let after = crate::threads::manager_for(&threads_path);
    assert!(
        !after
            .blocking_lock()
            .list()
            .iter()
            .any(|t| t.label == "secret plans"),
        "an agent recreated under this id would inherit deleted conversations"
    );
    drop(after);
    crate::threads::forget_managers_under(&root);
    std::fs::remove_dir_all(&root).ignore();
}

/// An agent another window still has open cannot be deleted out from under
/// it.
///
/// Evicting the cached manager does not reach a connection already holding
/// it, and that connection's next write recreates the directory — bringing
/// the conversations back, and leaving an agent later created under the same
/// id sharing a store with a manager nobody can see. Refusing is the only
/// answer that keeps one authority per store.
#[test]
fn an_agent_open_elsewhere_cannot_be_deleted() {
    use crate::agents::AgentRegistry;

    let root = std::env::temp_dir().join(format!("rustyclaw-agent-busy-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ignore();
    std::fs::create_dir_all(&root).unwrap();

    let registry = AgentRegistry::new(&root, "Main");
    registry
        .create(Some("busy"), "Busy", None, None, None)
        .expect("create");
    let threads_path = registry.agent_dir("busy").join("sessions/threads.json");

    // Something is holding the manager — a window, or a turn that outlived
    // one. The registry does not care which; holding it is what counts.
    let session = crate::threads::manager_for(&threads_path);
    let refused = registry.delete("busy");
    assert!(
        refused.is_err(),
        "deleting a store another session is using must be refused"
    );
    assert!(
        registry.agent_dir("busy").exists(),
        "and must not have removed anything"
    );

    // The last holder goes away; now it can be deleted.
    drop(session);
    registry
        .delete("busy")
        .expect("delete once nobody has it open");
    assert!(!registry.agent_dir("busy").exists());

    std::fs::remove_dir_all(&root).ignore();
}

// ── Background-process ownership (issue #231) ───────────────────────────────

/// A process one caller backgrounds is invisible and untouchable to another.
///
/// Exercised through the tool layer rather than `ProcessManager` directly, so
/// the caller identity travels the same task-local route it does in the
/// gateway — the part that was missing before.
#[tokio::test]
#[cfg(unix)]
async fn backgrounded_process_is_private_to_the_caller_that_started_it() {
    use crate::tool_caller::with_caller;
    use crate::tools::runtime::{exec_execute_command_streaming, exec_process_async};

    let spawned = with_caller("thread:owner", async {
        exec_execute_command_streaming(
            &json!({"command": "sleep 30", "background": true}),
            ws(),
            None,
        )
        .await
        .expect("background spawn")
    })
    .await;

    let session_id = serde_json::from_str::<Value>(&spawned)
        .ok()
        .and_then(|v| v["sessionId"].as_str().map(String::from))
        .expect("sessionId in spawn result");

    // The owner sees it listed and can address it.
    let owner_list = with_caller(
        "thread:owner",
        exec_process_async(&json!({"action": "list"}), ws()),
    )
    .await
    .expect("owner list");
    assert!(
        owner_list.contains(&session_id),
        "owner should see its own session, got: {owner_list}"
    );

    // Another conversation must not, even holding the id.
    let other_list = with_caller(
        "thread:other",
        exec_process_async(&json!({"action": "list"}), ws()),
    )
    .await
    .expect("other list");
    assert!(
        !other_list.contains(&session_id),
        "another thread must not see the session, got: {other_list}"
    );

    for action in ["poll", "log", "kill", "remove"] {
        let denied = with_caller(
            "thread:other",
            exec_process_async(&json!({"action": action, "sessionId": session_id}), ws()),
        )
        .await;
        let err = denied
            .expect_err(&format!("{action} from another thread must be refused"))
            .to_string();
        assert!(
            err.contains("No session found"),
            "refusal must not confirm the session exists, got: {err}"
        );
    }

    // Writing to another caller's stdin is the interference that matters most.
    let write_denied = with_caller(
        "thread:other",
        exec_process_async(
            &json!({"action": "write", "sessionId": session_id, "data": "x\n"}),
            ws(),
        ),
    )
    .await;
    assert!(
        write_denied.is_err(),
        "another thread must not write to the session's stdin"
    );

    // The owner can still clean up, proving the guard is not blanket denial.
    with_caller(
        "thread:owner",
        exec_process_async(&json!({"action": "remove", "sessionId": session_id}), ws()),
    )
    .await
    .expect("owner should still control its own session");
}

/// Without a caller scope a process is unowned, and stays shared — the
/// pre-ownership behaviour that direct CLI use and tests rely on.
#[tokio::test]
#[cfg(unix)]
async fn unidentified_callers_still_share_unowned_processes() {
    use crate::tools::runtime::{exec_execute_command_streaming, exec_process_async};

    let spawned = exec_execute_command_streaming(
        &json!({"command": "sleep 30", "background": true}),
        ws(),
        None,
    )
    .await
    .expect("background spawn");

    let session_id = serde_json::from_str::<Value>(&spawned)
        .ok()
        .and_then(|v| v["sessionId"].as_str().map(String::from))
        .expect("sessionId in spawn result");

    let seen = crate::tool_caller::with_caller(
        "thread:anyone",
        exec_process_async(&json!({"action": "poll", "sessionId": session_id}), ws()),
    )
    .await;
    assert!(
        seen.is_ok(),
        "an unowned session must stay reachable: {seen:?}"
    );

    exec_process_async(&json!({"action": "remove", "sessionId": session_id}), ws())
        .await
        .expect("cleanup");
}

// ── Concurrency ceilings (issue #232) ───────────────────────────────────────

/// A caller cannot accumulate background processes without limit.
///
/// The rate limit bounds how fast they start; this is the separate ceiling on
/// how many may be running at once, which is what a slow loop would otherwise
/// walk straight past.
#[tokio::test]
#[cfg(unix)]
async fn background_processes_are_capped_per_caller() {
    use crate::tool_caller::with_caller;
    use crate::tool_limits::{self, ToolLimitsConfig};
    use crate::tools::runtime::{exec_execute_command_streaming, exec_process_async};

    tool_limits::install(ToolLimitsConfig {
        max_background_processes: 2,
        ..Default::default()
    });

    async fn spawn_as(caller: &'static str) -> Result<String, crate::tools::error::ToolError> {
        with_caller(
            caller,
            exec_execute_command_streaming(
                &json!({"command": "sleep 30", "background": true}),
                ws(),
                None,
            ),
        )
        .await
    }

    fn session_id_of(out: &str) -> String {
        serde_json::from_str::<Value>(out)
            .ok()
            .and_then(|v| v["sessionId"].as_str().map(String::from))
            .expect("sessionId in spawn result")
    }

    // Every id this test creates, so cleanup touches only its own. Removing
    // whatever `list` returns would reach unowned sessions belonging to other
    // tests — those are shared by design.
    let mut mine: Vec<(&'static str, String)> = Vec::new();

    for _ in 0..2 {
        let out = spawn_as("thread:capped").await.expect("within the ceiling");
        mine.push(("thread:capped", session_id_of(&out)));
    }

    let err = spawn_as("thread:capped")
        .await
        .expect_err("a third background process must be refused")
        .to_string();
    assert!(
        err.contains("Too many background processes"),
        "refusal should say why, got: {err}"
    );

    // The ceiling is per caller: another caller is unaffected by this one
    // having filled its quota.
    let other = spawn_as("thread:elsewhere")
        .await
        .expect("another caller has its own ceiling");
    mine.push(("thread:elsewhere", session_id_of(&other)));

    // Freeing a slot lets the original caller start another.
    let (owner, first) = mine[0].clone();
    with_caller(
        owner,
        exec_process_async(&json!({"action": "remove", "sessionId": first}), ws()),
    )
    .await
    .expect("owner removes its own session");
    mine.remove(0);

    let after_free = spawn_as("thread:capped").await;
    assert!(
        after_free.is_ok(),
        "removing one should free a slot: {after_free:?}"
    );
    mine.push((
        "thread:capped",
        session_id_of(&after_free.expect("spawned")),
    ));

    for (caller, id) in mine {
        with_caller(
            caller,
            exec_process_async(&json!({"action": "remove", "sessionId": id}), ws()),
        )
        .await
        .ok();
    }

    tool_limits::install(ToolLimitsConfig::default());
}

/// `read_file` reaches Office documents on every platform (issue #144).
///
/// Before this, a .docx read as UTF-8 and failed outright anywhere but
/// macOS, where `textutil` picked it up. This asserts the routing, so the
/// regression would be caught rather than showing up as an unreadable file.
#[tokio::test]
async fn read_file_extracts_office_documents() {
    use crate::tools::file::exec_read_file_async;
    use std::io::Write;

    let dir = std::env::temp_dir().join("rustyclaw-readfile-docx");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("memo.docx");

    {
        let file = std::fs::File::create(&path).expect("create docx");
        let mut zip = zip::ZipWriter::new(file);
        let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        zip.start_file("word/document.xml", opts).expect("entry");
        zip.write_all(
            br#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
                <w:p><w:r><w:t>Board memo</w:t></w:r></w:p>
                </w:body></w:document>"#,
        )
        .expect("write");
        zip.finish().expect("finish");
    }

    let out = exec_read_file_async(&json!({"path": path.display().to_string()}), &dir)
        .await
        .expect("read_file should handle a docx");
    assert!(
        out.contains("Board memo"),
        "read_file must extract docx text, got: {out}"
    );

    std::fs::remove_file(&path).ok();
}
