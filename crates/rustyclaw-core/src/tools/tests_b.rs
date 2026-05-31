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
fn test_secrets_stub_rejects() {
    let args = json!({});
    let result = exec_secrets_stub(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("gateway"));
}

#[test]
fn test_is_secrets_tool() {
    assert!(is_secrets_tool("secrets_list"));
    assert!(is_secrets_tool("secrets_get"));
    assert!(is_secrets_tool("secrets_store"));
    assert!(!is_secrets_tool("read_file"));
    assert!(!is_secrets_tool("memory_get"));
}

#[test]
fn test_secrets_list_params_defined() {
    let params = secrets_list_params();
    assert_eq!(params.len(), 1);
    assert!(params.iter().any(|p| p.name == "prefix" && !p.required));
}

#[test]
fn test_secrets_get_params_defined() {
    let params = secrets_get_params();
    assert_eq!(params.len(), 1);
    assert!(params.iter().any(|p| p.name == "name" && p.required));
}

#[test]
fn test_secrets_store_params_defined() {
    let params = secrets_store_params();
    assert_eq!(params.len(), 6);
    assert!(params.iter().any(|p| p.name == "name" && p.required));
    assert!(params.iter().any(|p| p.name == "kind" && p.required));
    assert!(params.iter().any(|p| p.name == "value" && p.required));
    assert!(params.iter().any(|p| p.name == "policy" && !p.required));
    assert!(
        params
            .iter()
            .any(|p| p.name == "description" && !p.required)
    );
    assert!(params.iter().any(|p| p.name == "username" && !p.required));
}

#[test]
fn test_protected_path_without_init() {
    // Before set_credentials_dir is called, nothing is protected.
    assert!(!is_protected_path(Path::new("/some/random/path")));
}

// ── gateway ─────────────────────────────────────────────────────

#[test]
fn test_gateway_params_defined() {
    let params = gateway_params();
    assert_eq!(params.len(), 5);
    assert!(params.iter().any(|p| p.name == "action" && p.required));
}

#[test]
fn test_gateway_missing_action() {
    let args = json!({});
    let result = exec_gateway(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_gateway_config_schema() {
    let args = json!({ "action": "config.schema" });
    let result = exec_gateway(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("properties"));
}

// ── message ─────────────────────────────────────────────────────

#[test]
fn test_message_params_defined() {
    let params = message_params();
    assert_eq!(params.len(), 7);
    assert!(params.iter().any(|p| p.name == "action" && p.required));
}

#[test]
fn test_message_missing_action() {
    let args = json!({});
    let result = exec_message(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

// ── tts ─────────────────────────────────────────────────────────

#[test]
fn test_tts_params_defined() {
    let params = tts_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().any(|p| p.name == "text" && p.required));
}

#[test]
fn test_tts_missing_text() {
    let args = json!({});
    let result = exec_tts(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_tts_returns_media_path() {
    let args = json!({ "text": "Hello world" });
    let result = exec_tts(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("MEDIA:"));
}

// ── image ───────────────────────────────────────────────────────

#[test]
fn test_image_params_defined() {
    let params = image_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().any(|p| p.name == "image" && p.required));
    assert!(params.iter().any(|p| p.name == "prompt" && !p.required));
}

#[test]
fn test_image_missing_image() {
    let args = json!({});
    let result = exec_image(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_image_url_detection() {
    let args = json!({ "image": "https://example.com/photo.jpg" });
    let result = exec_image(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("Is URL: true"));
}

// ── nodes ───────────────────────────────────────────────────────

#[test]
fn test_nodes_params_defined() {
    let params = nodes_params();
    assert_eq!(params.len(), 8);
    assert!(params.iter().any(|p| p.name == "action" && p.required));
    assert!(params.iter().any(|p| p.name == "node" && !p.required));
}

#[test]
fn test_nodes_missing_action() {
    let args = json!({});
    let result = exec_nodes(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_nodes_status() {
    let args = json!({ "action": "status" });
    let result = exec_nodes(&args, ws());
    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.contains("nodes"));
    assert!(output.contains("tools"));
}

// ── browser ─────────────────────────────────────────────────────

#[test]
fn test_browser_params_defined() {
    let params = browser_params();
    assert_eq!(params.len(), 7);
    assert!(params.iter().any(|p| p.name == "action" && p.required));
}

#[test]
fn test_browser_missing_action() {
    let args = json!({});
    let result = exec_browser(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_browser_status() {
    let args = json!({ "action": "status" });
    let result = exec_browser(&args, ws());
    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.contains("running"));
}

// ── canvas ──────────────────────────────────────────────────────

#[test]
fn test_canvas_params_defined() {
    let params = canvas_params();
    assert_eq!(params.len(), 6);
    assert!(params.iter().any(|p| p.name == "action" && p.required));
}

#[test]
fn test_canvas_missing_action() {
    let args = json!({});
    let result = exec_canvas(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_canvas_snapshot() {
    let args = json!({ "action": "snapshot" });
    let result = exec_canvas(&args, ws());
    assert!(result.is_ok());
    let output = result.unwrap();
    // Without a URL presented first, snapshot returns no_canvas
    assert!(output.contains("no_canvas") || output.contains("snapshot_captured"));
}

// ── skill tools ─────────────────────────────────────────────────

#[test]
fn test_skill_list_params_defined() {
    let params = skill_list_params();
    assert_eq!(params.len(), 1);
    assert!(params.iter().any(|p| p.name == "filter" && !p.required));
}

#[test]
fn test_skill_search_params_defined() {
    let params = skill_search_params();
    assert_eq!(params.len(), 1);
    assert!(params.iter().any(|p| p.name == "query" && p.required));
}

#[test]
fn test_skill_install_params_defined() {
    let params = skill_install_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().any(|p| p.name == "name" && p.required));
    assert!(params.iter().any(|p| p.name == "version" && !p.required));
}

#[test]
fn test_skill_info_params_defined() {
    let params = skill_info_params();
    assert_eq!(params.len(), 1);
    assert!(params.iter().any(|p| p.name == "name" && p.required));
}

#[test]
fn test_skill_enable_params_defined() {
    let params = skill_enable_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().any(|p| p.name == "name" && p.required));
    assert!(params.iter().any(|p| p.name == "enabled" && p.required));
}

#[test]
fn test_skill_link_secret_params_defined() {
    let params = skill_link_secret_params();
    assert_eq!(params.len(), 3);
    assert!(params.iter().any(|p| p.name == "action" && p.required));
    assert!(params.iter().any(|p| p.name == "skill" && p.required));
    assert!(params.iter().any(|p| p.name == "secret" && p.required));
}

#[test]
fn test_skill_list_standalone_stub() {
    let result = exec_skill_list(&json!({}), ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("standalone mode"));
}

#[test]
fn test_skill_search_missing_query() {
    let result = exec_skill_search(&json!({}), ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_skill_install_missing_name() {
    let result = exec_skill_install(&json!({}), ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_skill_info_missing_name() {
    let result = exec_skill_info(&json!({}), ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_skill_enable_missing_params() {
    let result = exec_skill_enable(&json!({}), ws());
    assert!(result.is_err());
}

#[test]
fn test_skill_link_secret_bad_action() {
    let args = json!({ "action": "nope", "skill": "x", "secret": "y" });
    let result = exec_skill_link_secret(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unknown action"));
}

#[test]
fn test_is_skill_tool() {
    assert!(is_skill_tool("skill_list"));
    assert!(is_skill_tool("skill_search"));
    assert!(is_skill_tool("skill_install"));
    assert!(is_skill_tool("skill_info"));
    assert!(is_skill_tool("skill_enable"));
    assert!(is_skill_tool("skill_link_secret"));
    assert!(!is_skill_tool("read_file"));
    assert!(!is_skill_tool("secrets_list"));
}

// ── disk_usage ──────────────────────────────────────────────────

#[test]
fn test_disk_usage_params_defined() {
    let params = disk_usage_params();
    assert_eq!(params.len(), 3);
    assert!(params.iter().all(|p| !p.required));
}

#[test]
fn test_disk_usage_workspace() {
    let args = json!({ "path": ".", "depth": 1, "top": 5 });
    let result = exec_disk_usage(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("entries"));
}

#[test]
fn test_disk_usage_nonexistent() {
    let args = json!({ "path": "/nonexistent_path_xyz" });
    let result = exec_disk_usage(&args, ws());
    assert!(result.is_err());
}

// ── classify_files ──────────────────────────────────────────────

#[test]
fn test_classify_files_params_defined() {
    let params = classify_files_params();
    assert_eq!(params.len(), 1);
    assert!(params[0].required);
}

#[test]
fn test_classify_files_workspace() {
    let args = json!({ "path": "." });
    let result = exec_classify_files(&args, ws());
    assert!(result.is_ok());
    let text = result.unwrap();
    assert!(text.contains("path"));
}

#[test]
fn test_classify_files_missing_path() {
    let args = json!({});
    let result = exec_classify_files(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

// ── system_monitor ──────────────────────────────────────────────

#[test]
fn test_system_monitor_params_defined() {
    let params = system_monitor_params();
    assert_eq!(params.len(), 1);
    assert!(!params[0].required);
}

#[test]
fn test_system_monitor_all() {
    let args = json!({});
    let result = exec_system_monitor(&args, ws());
    assert!(result.is_ok());
}

#[test]
fn test_system_monitor_cpu() {
    let args = json!({ "metric": "cpu" });
    let result = exec_system_monitor(&args, ws());
    assert!(result.is_ok());
}

// ── battery_health ──────────────────────────────────────────────

#[test]
fn test_battery_health_params_defined() {
    let params = battery_health_params();
    assert_eq!(params.len(), 0);
}

#[test]
fn test_battery_health_runs() {
    let args = json!({});
    let result = exec_battery_health(&args, ws());
    assert!(result.is_ok());
}

// ── app_index ───────────────────────────────────────────────────

#[test]
fn test_app_index_params_defined() {
    let params = app_index_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().all(|p| !p.required));
}

#[test]
fn test_app_index_runs() {
    let args = json!({ "filter": "nonexistent_app_xyz" });
    let result = exec_app_index(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("apps"));
}

// ── cloud_browse ────────────────────────────────────────────────

#[test]
fn test_cloud_browse_params_defined() {
    let params = cloud_browse_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().all(|p| !p.required));
}

#[test]
fn test_cloud_browse_detect() {
    let args = json!({ "action": "detect" });
    let result = exec_cloud_browse(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("cloud_folders"));
}

#[test]
fn test_cloud_browse_invalid_action() {
    let args = json!({ "action": "invalid" });
    let result = exec_cloud_browse(&args, ws());
    assert!(result.is_err());
}

// ── browser_cache ───────────────────────────────────────────────

#[test]
fn test_browser_cache_params_defined() {
    let params = browser_cache_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().all(|p| !p.required));
}

#[test]
fn test_browser_cache_scan() {
    let args = json!({ "action": "scan" });
    let result = exec_browser_cache(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("caches"));
}

// ── screenshot ──────────────────────────────────────────────────

#[test]
fn test_screenshot_params_defined() {
    let params = screenshot_params();
    assert_eq!(params.len(), 3);
    assert!(params.iter().all(|p| !p.required));
}

// ── clipboard ───────────────────────────────────────────────────

#[test]
fn test_clipboard_params_defined() {
    let params = clipboard_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().any(|p| p.name == "action" && p.required));
}

#[test]
fn test_clipboard_missing_action() {
    let args = json!({});
    let result = exec_clipboard(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_clipboard_invalid_action() {
    let args = json!({ "action": "invalid" });
    let result = exec_clipboard(&args, ws());
    assert!(result.is_err());
}

// ── audit_sensitive ─────────────────────────────────────────────

#[test]
fn test_audit_sensitive_params_defined() {
    let params = audit_sensitive_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().all(|p| !p.required));
}

#[test]
fn test_audit_sensitive_runs() {
    let dir = std::env::temp_dir().join("rustyclaw_test_audit");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("safe.txt"), "nothing sensitive here").unwrap();
    let args = json!({ "path": ".", "max_files": 10 });
    let result = exec_audit_sensitive(&args, &dir);
    assert!(result.is_ok());
    assert!(result.unwrap().contains("scanned_files"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ── secure_delete ───────────────────────────────────────────────

#[test]
fn test_secure_delete_params_defined() {
    let params = secure_delete_params();
    assert_eq!(params.len(), 3);
    assert!(params.iter().any(|p| p.name == "path" && p.required));
}

#[test]
fn test_secure_delete_missing_path() {
    let args = json!({});
    let result = exec_secure_delete(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_secure_delete_nonexistent() {
    let args = json!({ "path": "/tmp/nonexistent_rustyclaw_xyz" });
    let result = exec_secure_delete(&args, ws());
    assert!(result.is_err());
}

#[test]
fn test_secure_delete_requires_confirm() {
    let dir = std::env::temp_dir().join("rustyclaw_test_secdelete");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("victim.txt"), "data").unwrap();
    let args = json!({ "path": dir.join("victim.txt").display().to_string() });
    let result = exec_secure_delete(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("confirm_required"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_secure_delete_with_confirm() {
    let dir = std::env::temp_dir().join("rustyclaw_test_secdelete2");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let victim = dir.join("victim.txt");
    std::fs::write(&victim, "secret data").unwrap();
    let args = json!({
        "path": victim.display().to_string(),
        "confirm": true,
    });
    let result = exec_secure_delete(&args, ws());
    assert!(result.is_ok());
    assert!(result.unwrap().contains("deleted"));
    assert!(!victim.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

// ── summarize_file ──────────────────────────────────────────────

#[test]
fn test_summarize_file_params_defined() {
    let params = summarize_file_params();
    assert_eq!(params.len(), 2);
    assert!(params.iter().any(|p| p.name == "path" && p.required));
}

#[test]
fn test_summarize_file_missing_path() {
    let args = json!({});
    let result = exec_summarize_file(&args, ws());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing required parameter"));
}

#[test]
fn test_summarize_file_this_file() {
    let args = json!({ "path": file!(), "max_lines": 10 });
    let result = exec_summarize_file(&args, ws());
    assert!(result.is_ok());
    let text = result.unwrap();
    assert!(text.contains("text"));
    assert!(text.contains("total_lines"));
}

#[test]
fn test_summarize_file_nonexistent() {
    let args = json!({ "path": "/nonexistent/file.txt" });
    let result = exec_summarize_file(&args, ws());
    assert!(result.is_err());
}
