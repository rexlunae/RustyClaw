//! Support functions for the desktop `App` component: gateway connection,
//! gateway-event application, DOM queries, directory helpers, and swarm ops.

use std::sync::Arc;

use dioxus::prelude::*;
use rustyclaw_view::{chrono, serde_json, tracing, uuid};

use crate::state::AppState;
use rustyclaw_core::gateway::GatewayClient;
use rustyclaw_core::gateway::client_types::{GatewayCommand, GatewayEvent};
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::{ConnectionStatus, ThreadInfo};
use rustyclaw_view::{SecretsDialogData, SwarmAgentData, SwarmData};

// ── Shared buffer for the worker → UI bridge ───────────────────────────────

/// An entry in the ordered event buffer.  Consecutive Chunk events
/// are coalesced into a single `Chunks` entry to reduce signal writes,
/// while preserving the ordering of non-chunk events relative to chunks.
pub(crate) enum BufferEntry {
    Event(GatewayEvent),
    Chunks {
        text: String,
        count: u32,
        bytes: usize,
    },
}

/// Intermediate buffer between the tokio event-consumer worker and
/// the Dioxus UI task.  The worker writes at full speed; the UI task
/// drains on each `Notify` wake-up.
#[derive(Default)]
pub(crate) struct EventBuffer {
    pub(crate) entries: Vec<BufferEntry>,
}

/// Connect to the gateway.
pub(crate) async fn connect_to_gateway_candidates(
    urls: Vec<String>,
    mut state: Signal<AppState>,
    gateway: Signal<Option<Arc<GatewayClient>>>,
) -> bool {
    for url in urls {
        state.write().gateway_url = url.clone();
        connect_to_gateway(&url, state, gateway).await;
        if matches!(
            state.read().connection,
            ConnectionStatus::Connected | ConnectionStatus::Authenticated
        ) {
            crate::save_gateway_url(&url);
            return true;
        }
    }
    false
}

/// Connect to the gateway.
pub(crate) async fn connect_to_gateway(
    url: &str,
    mut state: Signal<AppState>,
    mut gateway: Signal<Option<Arc<GatewayClient>>>,
) {
    state.write().connection = ConnectionStatus::Connecting;

    match GatewayClient::connect(url).await {
        Ok(client) => {
            gateway.set(Some(Arc::new(client)));
            state.write().connection = ConnectionStatus::Connected;
        }
        Err(e) => {
            state.write().connection = ConnectionStatus::Error(e.to_string());
            tracing::error!("Failed to connect to gateway: {}", e);
        }
    }
}

/// Handle a gateway event.
pub(crate) fn handle_gateway_event(event: GatewayEvent, mut state: Signal<AppState>) {
    match event {
        GatewayEvent::Connected {
            agent,
            vault_locked,
            provider,
            model,
        } => {
            let mut s = state.write();
            s.connection = ConnectionStatus::Connected;
            s.agent_name = agent;
            s.vault_locked = vault_locked;
            s.provider = provider.map(|p| normalize_provider_id(&p).to_string());
            s.model = model;
            // A fresh session has nothing in flight; clear any indicator
            // state left over from a request the old connection dropped, so
            // it can't block history hydration or show a phantom spinner.
            s.is_processing = false;
            s.is_streaming = false;
            s.is_thinking = false;
            s.streaming_thread_id = None;
        }
        GatewayEvent::Disconnected { reason } => {
            let mut s = state.write();
            s.connection = ConnectionStatus::Disconnected;
            if let Some(r) = reason {
                s.push_notice(MessageRole::Warning, format!("Disconnected: {}", r));
            }
            // The in-flight request (if any) died with the connection.
            s.is_processing = false;
            s.is_streaming = false;
            s.is_thinking = false;
            s.streaming_thread_id = None;
        }
        GatewayEvent::AuthRequired => {
            state.write().connection = ConnectionStatus::Authenticating;
        }
        GatewayEvent::AuthSuccess => {
            state.write().connection = ConnectionStatus::Authenticated;
        }
        GatewayEvent::AuthFailed { message, retry } => {
            let text = if retry {
                format!("Auth failed (retry allowed): {}", message)
            } else {
                format!("Auth failed: {}", message)
            };
            state.write().push_notice(MessageRole::Error, text);
        }
        GatewayEvent::VaultLocked => {
            state.write().vault_locked = true;
        }
        GatewayEvent::VaultUnlocked => {
            state.write().vault_locked = false;
        }
        GatewayEvent::ModelReady { model } => {
            state.write().model = Some(model);
        }
        GatewayEvent::ModelError { message } => {
            state
                .write()
                .push_notice(MessageRole::Error, format!("Model error: {}", message));
        }
        // Live stream events carry no thread id; they belong to the thread
        // that submitted the request (`streaming_thread_id`). When the user
        // has switched away from it, they must not touch the on-screen view
        // or its indicators — the backgrounded thread's transcript arrives
        // via the gateway's history snapshot on completion.
        GatewayEvent::StreamStart => {
            let mut s = state.write();
            if s.stream_targets_foreground() {
                s.start_assistant_message();
            }
        }
        GatewayEvent::ThinkingStart => {
            let mut s = state.write();
            if s.stream_targets_foreground() {
                s.is_thinking = true;
            }
        }
        GatewayEvent::ThinkingEnd => {
            let mut s = state.write();
            if s.stream_targets_foreground() {
                s.is_thinking = false;
            }
        }
        GatewayEvent::Chunk { delta } => {
            let mut s = state.write();
            if s.stream_targets_foreground() {
                s.append_to_current_message(&delta);
            }
        }
        GatewayEvent::ResponseDone => {
            state.write().response_done();
        }
        GatewayEvent::ToolCall {
            id,
            name,
            arguments,
        } => {
            let mut s = state.write();
            if s.stream_targets_foreground() {
                s.add_tool_call(id, name, arguments);
                // A tool call marks the end of this round's text stream; the
                // gateway is now executing the tool. Switch the indicator from
                // "Streaming…" (which would sit frozen) to the processing bar
                // while the tool panel shows the running call. `is_processing`
                // stays set until ResponseDone.
                s.is_streaming = false;
            }
        }
        GatewayEvent::ToolResult {
            id,
            name: _,
            result,
            is_error,
        } => {
            let mut s = state.write();
            // A failed tool call already surfaces inline: the tool panel
            // shows Failed status with the full error result. No banner.
            if s.stream_targets_foreground() {
                s.set_tool_result(&id, result, is_error);
            }
        }
        GatewayEvent::ToolApprovalRequest {
            id,
            name,
            arguments,
        } => {
            state.write().pending_tool_approval = Some((id, name, arguments));
        }
        GatewayEvent::ThreadsUpdate {
            threads,
            foreground_id,
        } => {
            let count = threads.len();
            let captions = threads
                .iter()
                .map(|t| format!("{}:{}", t.id, t.label.as_deref().unwrap_or("")))
                .collect::<Vec<_>>();
            tracing::info!(
                count,
                foreground_id = ?foreground_id,
                captions = ?captions,
                "ThreadsUpdate received"
            );
            state.write().threads = threads
                .into_iter()
                .map(|t| ThreadInfo {
                    id: t.id,
                    project_id: t.project_id,
                    label: t.label,
                    description: t.description,
                    status: t.status,
                    is_foreground: t.is_foreground,
                    message_count: t.message_count,
                })
                .collect();
            state.write().foreground_thread_id = foreground_id;
        }
        GatewayEvent::ProjectsUpdate {
            projects,
            active_id,
        } => {
            state.write().projects = projects
                .into_iter()
                .map(|p| rustyclaw_core::ui::ProjectInfo {
                    id: p.id,
                    name: p.name,
                    path: p.path,
                })
                .collect();
            state.write().active_project_id = active_id;
        }
        GatewayEvent::ThreadHistory {
            thread_id,
            ok,
            messages,
            error,
        } => {
            if !ok {
                if let Some(err) = error {
                    tracing::warn!(
                        thread_id,
                        error = %err,
                        "ThreadHistory request failed"
                    );
                }
            } else {
                tracing::info!(
                    thread_id,
                    incoming_messages = messages.len(),
                    foreground = ?state.read().foreground_thread_id,
                    "Desktop thread history reply received"
                );
                use rustyclaw_core::types::MessageRole;
                use rustyclaw_core::ui::{ChatMessage as UiChatMessage, ToolCallInfo};
                use std::collections::VecDeque;
                let mut converted: VecDeque<UiChatMessage> =
                    VecDeque::with_capacity(messages.len());
                for m in messages.into_iter() {
                    // Tool result: fold into the previous assistant turn's
                    // matching tool call rather than emit a standalone bubble.
                    if m.role == "tool"
                        && let Some(call_id) = m.tool_call_id.as_deref()
                        && let Some(prev) = converted.iter_mut().rev().find(|c| {
                            c.role == MessageRole::Assistant
                                && c.tool_calls.iter().any(|tc| tc.id == call_id)
                        })
                    {
                        if let Some(tc) = prev.tool_calls.iter_mut().find(|tc| tc.id == call_id) {
                            tc.result = Some(m.content.clone());
                            tc.is_error = false;
                        }
                        continue;
                    }
                    let role = match m.role.as_str() {
                        "user" => MessageRole::User,
                        "assistant" => MessageRole::Assistant,
                        "tool" => MessageRole::ToolResult,
                        "system" => MessageRole::System,
                        _ => MessageRole::System,
                    };
                    let mut tool_calls: Vec<ToolCallInfo> = Vec::new();
                    if let Some(tcs) = m.tool_calls.as_ref().and_then(|v| v.as_array()) {
                        for tc in tcs {
                            tool_calls.push(ToolCallInfo {
                                id: tc
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                name: tc
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                arguments: tc
                                    .get("arguments")
                                    .map(|v| v.to_string())
                                    .unwrap_or_default(),
                                result: None,
                                is_error: false,
                                collapsed: true,
                            });
                        }
                    }
                    converted.push_back(UiChatMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        role,
                        content: m.content,
                        timestamp: chrono::Utc::now(),
                        tool_calls,
                        is_streaming: false,
                    });
                }
                tracing::info!(
                    thread_id,
                    converted_messages = converted.len(),
                    "Desktop thread history converted"
                );
                state.write().apply_thread_history(thread_id, converted);
            }
        }
        GatewayEvent::ThreadMessages {
            thread_id,
            messages,
        } => {
            state.write().hydrate_thread_messages(thread_id, messages);
        }
        GatewayEvent::UserPromptRequest { id: _, prompt } => {
            state.write().pending_user_prompt = Some(prompt);
        }
        GatewayEvent::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        } => {
            state.write().pending_credential_request = Some((id, provider, secret_name, message));
        }
        GatewayEvent::DeviceFlowStart { url, code, message } => {
            state.write().pending_device_flow = Some((url, code, message));
        }
        GatewayEvent::DeviceFlowComplete => {
            state.write().pending_device_flow = None;
        }
        GatewayEvent::SecretsListResult { ok, entries } => {
            if ok {
                let data = SecretsDialogData::from_vault(
                    entries.iter().map(Into::into).collect(),
                    state.read().agent_access,
                    false, // has_totp — would need gateway peek
                );
                state.write().secrets_data = data;
            } else {
                state
                    .write()
                    .push_notice(MessageRole::Error, "Failed to list secrets.");
            }
        }
        GatewayEvent::SecretsStoreResult { ok, message } => {
            if ok {
                state
                    .write()
                    .push_notice(MessageRole::Success, "Secret stored successfully.");
                // Trigger refresh to show new secret
                // (parent doesn't have gateway handle here, so we just update status)
            } else {
                state.write().push_notice(
                    MessageRole::Error,
                    format!("Failed to store secret: {}", message),
                );
            }
        }
        GatewayEvent::SecretsDeleteResult { ok, message } => {
            if ok {
                state
                    .write()
                    .push_notice(MessageRole::Success, "Secret deleted.");
            } else {
                state.write().push_notice(
                    MessageRole::Error,
                    format!("Failed to delete secret: {}", message.unwrap_or_default()),
                );
            }
        }
        GatewayEvent::SecretsSetPolicyResult { ok, message } => {
            if ok {
                state
                    .write()
                    .push_notice(MessageRole::Success, "Policy updated.");
            } else {
                state.write().push_notice(
                    MessageRole::Error,
                    format!("Failed to set policy: {}", message.unwrap_or_default()),
                );
            }
        }
        GatewayEvent::ModelReloaded { provider, model } => {
            state.write().push_notice(
                MessageRole::Success,
                format!("Model reloaded: {provider}/{model}"),
            );
        }
        GatewayEvent::ThinkingDelta => {
            // Keeps the thinking clock alive server-side; the desktop tracks
            // thinking state via ThinkingStart/ThinkingEnd, so nothing to do.
        }
        GatewayEvent::ThreadSwitched { .. } => {
            // Thread state syncs via ThreadsUpdate/ThreadHistory.
        }
        GatewayEvent::Warning { message } => {
            state.write().push_notice(MessageRole::Warning, message);
        }
        // Secrets-management results without a desktop UI surface. These are
        // driven from the TUI's secrets manager; the desktop ignores them.
        GatewayEvent::SecretsGetResult { .. }
        | GatewayEvent::SecretsPeekResult { .. }
        | GatewayEvent::SecretsSetDisabledResult { .. }
        | GatewayEvent::SecretsDeleteCredentialResult { .. }
        | GatewayEvent::SecretsHasTotpResult { .. }
        | GatewayEvent::SecretsSetupTotpResult { .. }
        | GatewayEvent::SecretsVerifyTotpResult { .. }
        | GatewayEvent::SecretsRemoveTotpResult { .. } => {}
        GatewayEvent::Error { message } => {
            let mut s = state.write();
            s.push_notice(MessageRole::Error, message);
            s.is_processing = false;
        }
        GatewayEvent::Info { message } => {
            state.write().push_notice(MessageRole::Info, message);
        }
        GatewayEvent::DomQuery { .. } => {
            // Handled directly in the UI updater task via handle_dom_query.
        }
        GatewayEvent::HostInfo {
            hostname,
            os,
            arch,
            cpu_brand,
            cpu_cores_physical,
            cpu_cores_logical,
            cpu_frequency_mhz,
            total_memory_bytes,
            total_swap_bytes,
            disk_total_bytes,
            disk_available_bytes,
            gpus,
            summary,
        } => {
            let gib = |b: u64| b as f64 / (1024.0 * 1024.0 * 1024.0);
            state.write().host_info = Some(rustyclaw_view::HostInfoData {
                hostname,
                os,
                arch,
                cpu_brand,
                cpu_cores_physical,
                cpu_cores_logical,
                cpu_frequency_mhz,
                total_memory_gib: gib(total_memory_bytes),
                total_swap_gib: gib(total_swap_bytes),
                disk_total_gib: gib(disk_total_bytes),
                disk_available_gib: gib(disk_available_bytes),
                gpus: gpus
                    .into_iter()
                    .map(|g| rustyclaw_view::GpuDisplayInfo {
                        name: g.name,
                        vendor: g.vendor,
                        vram_gib: gib(g.vram_bytes),
                    })
                    .collect(),
                summary,
            });
        }
        GatewayEvent::LoadStatus {
            load_score,
            avg_load_score,
            cpu_percent,
            memory_percent,
            summary,
        } => {
            state.write().load_status = Some(rustyclaw_view::LoadStatusData {
                load_score,
                avg_load_score,
                cpu_percent,
                memory_percent,
                summary,
            });
        }
        GatewayEvent::ServiceList { services } => {
            state.write().services_data = Some(rustyclaw_view::ServiceListData {
                services: services.into_iter().map(Into::into).collect(),
            });
        }
        GatewayEvent::ServiceActionResult { service, .. } => {
            if let Some(svc) = service {
                let info = rustyclaw_view::ServiceInfoData::from(svc);
                let mut st = state.write();
                if let Some(ref mut data) = st.services_data {
                    if let Some(existing) = data.services.iter_mut().find(|s| s.name == info.name) {
                        *existing = info;
                    } else {
                        data.services.push(info);
                    }
                }
            }
        }
        GatewayEvent::ServiceLogs { .. } => {
            // Logs are displayed in a separate dialog; no state update needed.
        }
        // ── Engines ──────────────────────────────────────────────────────
        GatewayEvent::EngineListResult { .. }
        | GatewayEvent::EngineModelListResult { .. }
        | GatewayEvent::EnginePullProgress { .. } => {
            // Will be rendered in the engines dialog panel in P5.
        }
        GatewayEvent::EngineActionResult { ok, message, .. } => {
            let mut s = state.write();
            if ok {
                s.push_notice(MessageRole::Success, format!("Engine: {}", message));
            } else {
                s.push_notice(MessageRole::Error, format!("Engine error: {}", message));
            }
        }
    }
}

pub(crate) fn normalize_provider_id(id: &str) -> &str {
    match id {
        "copilot" | "github_copilot" | "githubcopilot" => "github-copilot",
        other => other,
    }
}

// ── DOM query handler ───────────────────────────────────────────────────────

/// Execute a JavaScript expression in the webview and send the result
/// back to the gateway as a `DomQueryResponse`.
pub(crate) async fn handle_dom_query(client: &Arc<GatewayClient>, id: String, js: String) {
    let wrapped = format!(
        r#"(function() {{
            try {{
                var __result = (function() {{ return {js}; }})();
                if (typeof __result === 'undefined') return JSON.stringify({{__ok:true,__v:'undefined'}});
                if (typeof __result === 'string') return JSON.stringify({{__ok:true,__v:__result}});
                return JSON.stringify({{__ok:true,__v:JSON.stringify(__result)}});
            }} catch(e) {{
                return JSON.stringify({{__ok:false,__v:e.message}});
            }}
        }})()"#,
    );

    // Retry up to 3 times to work around Dioxus EvalError::Finished
    // bug (https://github.com/DioxusLabs/dioxus/issues/3084) where
    // eval sometimes reports "already ran" spuriously.
    let mut attempts = 0;
    let (result, is_error) = loop {
        attempts += 1;
        let eval = document::eval(&wrapped);
        match eval.await {
            Ok(val) => {
                let raw = match val {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                break match serde_json::from_str::<serde_json::Value>(&raw) {
                    Ok(obj) => {
                        let ok = obj.get("__ok").and_then(|v| v.as_bool()).unwrap_or(false);
                        let v = obj
                            .get("__v")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        (v, !ok)
                    }
                    Err(_) => (raw, false),
                };
            }
            Err(e) => {
                if attempts < 3 {
                    tracing::warn!(attempt = attempts, error = %e, "DOM eval failed, retrying");
                    continue;
                }
                break (
                    format!("eval error after {} attempts: {}", attempts, e),
                    true,
                );
            }
        }
    };

    let _ = client
        .send(GatewayCommand::DomQueryResponse {
            id,
            result,
            is_error,
        })
        .await;
}

pub(crate) fn display_path(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME")
        && path.starts_with(&home)
    {
        return path.replacen(&home, "~", 1);
    }
    path.to_string()
}

pub(crate) fn build_directory_options(base_path: &str) -> Vec<rustyclaw_view::DirectoryOption> {
    use std::path::Path;

    let mut options: Vec<rustyclaw_view::DirectoryOption> = Vec::new();
    let base = Path::new(base_path);

    options.push(rustyclaw_view::DirectoryOption {
        path: base_path.to_string(),
        display_name: display_path(base_path),
        is_selected: true,
    });

    if let Some(parent) = base.parent() {
        let parent_str = parent.display().to_string();
        options.push(rustyclaw_view::DirectoryOption {
            path: parent_str.clone(),
            display_name: format!("../ ({})", display_path(&parent_str)),
            is_selected: false,
        });
    }

    if let Ok(home) = std::env::var("HOME")
        && home != base_path
    {
        options.push(rustyclaw_view::DirectoryOption {
            path: home.clone(),
            display_name: "Home (~)".to_string(),
            is_selected: false,
        });
    }

    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.filter_map(Result::ok).take(24) {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let p = entry.path().display().to_string();
            if p == base_path {
                continue;
            }
            let label = entry.file_name().to_string_lossy().to_string();
            options.push(rustyclaw_view::DirectoryOption {
                path: p,
                display_name: label,
                is_selected: false,
            });
        }
    }

    options
}

// ── Swarm helpers ───────────────────────────────────────────────────────────

/// Build the current list of swarm infos from the global swarm manager.
pub(crate) fn get_swarm_infos() -> Vec<SwarmData> {
    use rustyclaw_core::swarm::swarm_manager;

    let mgr = match swarm_manager().lock() {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    mgr.list()
        .into_iter()
        .map(|inst| SwarmData {
            name: inst.config.name.clone(),
            status: inst.status.to_string(),
            description: inst.config.description.clone(),
            tasks_routed: inst.tasks_routed,
            uptime_secs: inst.runtime_secs(),
            agents: inst
                .config
                .agents
                .iter()
                .map(|a| SwarmAgentData {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    role: a.role.to_string(),
                    description: a.description.clone(),
                    has_session: inst.agent_sessions.contains_key(&a.id),
                })
                .collect(),
        })
        .collect()
}

/// Create a swarm from a built-in template.
pub(crate) fn create_swarm_from_template(template: &str) -> Result<(), String> {
    use rustyclaw_core::swarm::{builtin_templates, swarm_manager};

    let templates = builtin_templates();
    let cfg = templates
        .into_iter()
        .find(|t| t.name == template)
        .ok_or_else(|| format!("Unknown template: {}", template))?;

    let name = cfg.name.clone();
    let mgr = swarm_manager();
    let mut m = mgr.lock().map_err(|_| "Lock error".to_string())?;
    m.create(cfg).map_err(|e| e.to_string())?;
    m.start(&name).map_err(|e| e.to_string())?;
    Ok(())
}

/// Stop a running swarm.
pub(crate) fn stop_swarm(name: &str) -> Result<(), String> {
    use rustyclaw_core::swarm::swarm_manager;

    let mgr = swarm_manager();
    let mut m = mgr.lock().map_err(|_| "Lock error".to_string())?;
    m.stop(name).map_err(|e| e.to_string())
}
