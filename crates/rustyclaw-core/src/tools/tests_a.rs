//! Tests for the tools module.

#![allow(unused_imports, dead_code)]
use super::*;
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
    assert!(result.unwrap_err().contains("Missing required parameter"));
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
    let _ = std::fs::remove_dir_all(&dir);
    let args = json!({
        "path": "sub/test.txt",
        "content": "hello world"
    });
    let result = exec_write_file(&args, &dir);
    assert!(result.is_ok());
    assert!(result.unwrap().contains("11 bytes"));

    let content = std::fs::read_to_string(dir.join("sub/test.txt")).unwrap();
    assert_eq!(content, "hello world");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── edit_file ───────────────────────────────────────────────────

#[test]
fn test_edit_file_single_match() {
    let dir = std::env::temp_dir().join("rustyclaw_test_edit");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.txt"), "aaa\nbbb\nccc\n").unwrap();

    let args = json!({ "path": "f.txt", "old_string": "bbb", "new_string": "BBB" });
    let result = exec_edit_file(&args, &dir);
    assert!(result.is_ok());

    let content = std::fs::read_to_string(dir.join("f.txt")).unwrap();
    assert_eq!(content, "aaa\nBBB\nccc\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_edit_file_no_match() {
    let dir = std::env::temp_dir().join("rustyclaw_test_edit_no");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.txt"), "aaa\nbbb\n").unwrap();

    let args = json!({ "path": "f.txt", "old_string": "zzz", "new_string": "ZZZ" });
    let result = exec_edit_file(&args, &dir);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_edit_file_multiple_matches() {
    let dir = std::env::temp_dir().join("rustyclaw_test_edit_multi");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.txt"), "aaa\naaa\n").unwrap();

    let args = json!({ "path": "f.txt", "old_string": "aaa", "new_string": "bbb" });
    let result = exec_edit_file(&args, &dir);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("2 times"));
    let _ = std::fs::remove_dir_all(&dir);
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
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "hello world\n").unwrap();

    let args = json!({ "pattern": "XYZZY_NEVER_42" });
    let result = exec_search_files(&args, &dir);
    assert!(result.is_ok());
    assert!(result.unwrap().contains("No matches"));
    let _ = std::fs::remove_dir_all(&dir);
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
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hello.txt"), "content").unwrap();

    let args = json!({ "pattern": "resume" });
    let result = exec_find_files(&args, &dir);
    assert!(result.is_ok());
    assert!(result.unwrap().contains("No files found"));
    let _ = std::fs::remove_dir_all(&dir);
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
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_web_fetch_invalid_url() {
    let args = json!({ "url": "not-a-url" });
    let result = exec_web_fetch(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("http"));
}

#[test]
fn test_web_fetch_params_defined() {
    let params = web_fetch_params();
    assert_eq!(params.len(), 6);
    assert!(params.iter().any(|p| p.name == "url" && p.required));
    assert!(
        params
            .iter()
            .any(|p| p.name == "extract_mode" && !p.required)
    );
    assert!(params.iter().any(|p| p.name == "max_chars" && !p.required));
    assert!(
        params
            .iter()
            .any(|p| p.name == "use_cookies" && !p.required)
    );
    assert!(
        params
            .iter()
            .any(|p| p.name == "authorization" && !p.required)
    );
    assert!(params.iter().any(|p| p.name == "headers" && !p.required));
}

// ── web_search ──────────────────────────────────────────────────

#[test]
fn test_web_search_missing_query() {
    let args = json!({});
    let result = exec_web_search(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_web_search_no_api_key() {
    // Clear any existing key for the test
    // SAFETY: This test is single-threaded and no other thread reads BRAVE_API_KEY.
    unsafe { std::env::remove_var("BRAVE_API_KEY") };
    let args = json!({ "query": "test" });
    let result = exec_web_search(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("BRAVE_API_KEY"));
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
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_process_invalid_action() {
    let args = json!({ "action": "invalid" });
    let result = exec_process(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unknown action"));
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

#[test]
fn test_memory_search_missing_query() {
    let args = json!({});
    let result = exec_memory_search(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
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
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_memory_get_invalid_path() {
    let args = json!({ "path": "../etc/passwd" });
    let result = exec_memory_get(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not a valid memory file"));
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
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_cron_invalid_action() {
    let args = json!({ "action": "invalid" });
    let result = exec_cron(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unknown action"));
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
    assert_eq!(params.len(), 7);
    assert!(params.iter().any(|p| p.name == "task" && p.required));
}

#[test]
fn test_sessions_spawn_missing_task() {
    let args = json!({});
    let result = exec_sessions_spawn(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
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
    assert!(result.unwrap_err().contains("Missing required parameter"));
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
    assert!(result.unwrap_err().contains("Missing required parameter"));
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
