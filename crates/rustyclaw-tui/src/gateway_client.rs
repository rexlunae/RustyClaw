//! Client-side protocol adapter.
//!
//! Wire-frame parsing is shared in `rustyclaw_core::gateway`
//! ([`GatewayEvent::from_server_frame`]); the core [`GatewayClient`] produces
//! [`GatewayEvent`]s, and this module adapts those into the TUI's UI-level
//! [`GwEvent`]s (dialog prompts, render updates, status messages). It is the
//! single translation between the shared event model and the TUI.
//!
//! [`GatewayClient`]: rustyclaw_core::gateway::GatewayClient

use crate::app::{GwEvent, PanelKind};
use rustyclaw_core::gateway::GatewayEvent;

/// Adapt a shared gateway event into a TUI UI event.
///
/// Returns `None` for events the TUI does not surface (e.g. DOM queries, which
/// require a webview the TUI does not have).
pub(crate) fn gateway_event_to_gw_event(
    thread_id: Option<u64>,
    event: GatewayEvent,
) -> Option<GwEvent> {
    use GatewayEvent as E;

    let ev = match event {
        // ── Connection lifecycle ────────────────────────────────────────
        E::Connected { .. } => GwEvent::Connected,
        E::Disconnected { reason } => GwEvent::Disconnected(reason.unwrap_or_default()),
        E::AuthRequired => GwEvent::AuthChallenge,
        E::AuthSuccess => GwEvent::Authenticated,
        E::AuthFailed { message, retry } => {
            if retry {
                GwEvent::warning(if message.is_empty() {
                    "Invalid code. Try again.".to_string()
                } else {
                    message
                })
            } else {
                GwEvent::error(if message.is_empty() {
                    "Authentication failed.".to_string()
                } else {
                    message
                })
            }
        }
        E::VaultLocked => GwEvent::VaultLocked,
        E::VaultUnlocked => GwEvent::VaultUnlocked,

        // ── Model status ────────────────────────────────────────────────
        // `ModelReady` updates the status-bar model label, so it maps to the
        // dedicated event rather than a transient success toast.
        E::ModelReady { model } => GwEvent::ModelReady(model),
        E::ModelError { message } => GwEvent::error(message),
        E::ModelReloaded { provider, model } => GwEvent::ModelReloaded { provider, model },

        // ── Streaming ───────────────────────────────────────────────────
        E::StreamStart { thread_id } => GwEvent::StreamStart(thread_id),
        E::ThinkingStart => GwEvent::ThinkingStart(thread_id),
        E::ThinkingDelta { delta } => GwEvent::ThinkingDelta(thread_id, delta),
        E::ThinkingEnd => GwEvent::ThinkingEnd(thread_id),
        // Tagged with the turn that produced it. The TUI renders one thread
        // at a time and shares a single streaming buffer, so a chunk from a
        // turn running elsewhere has to be recognisable — appending it here
        // would splice two answers together.
        E::Chunk { delta } => GwEvent::Chunk(thread_id, delta),
        E::ResponseDone { thread_id } => GwEvent::ResponseDone(thread_id),

        // ── Tool calls ──────────────────────────────────────────────────
        E::ToolCall {
            id,
            name,
            arguments,
        } => GwEvent::ToolCall {
            thread_id,
            id,
            name,
            arguments,
        },
        E::ToolResult {
            id,
            name,
            result,
            is_error,
        } => GwEvent::ToolResult {
            thread_id,
            id,
            name,
            result,
            is_error,
        },
        E::ToolStatus {
            id,
            name: _,
            elapsed_ms,
            pid,
            cpu_percent,
            memory_bytes,
            state,
            message,
        } => GwEvent::ToolStatus {
            thread_id,
            id,
            status: rustyclaw_core::ui::ToolLiveStatus {
                elapsed_ms,
                pid,
                cpu_percent,
                memory_bytes,
                state,
                message,
            },
        },
        // stderr chunks merge into the same tail a terminal would show;
        // the flag isn't currently surfaced in either client.
        E::ToolOutput { id, chunk, .. } => GwEvent::ToolOutput {
            thread_id,
            id,
            chunk,
        },
        E::ToolApprovalRequest {
            id,
            name,
            arguments,
        } => GwEvent::ToolApprovalRequest {
            thread_id,
            id,
            name,
            arguments,
        },

        // ── Interactive prompts ─────────────────────────────────────────
        E::UserPromptRequest { prompt, .. } => GwEvent::UserPromptRequest { thread_id, prompt },
        E::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        } => GwEvent::CredentialRequest {
            thread_id,
            id,
            provider,
            secret_name,
            message,
        },
        E::DeviceFlowStart { url, code, .. } => GwEvent::DeviceFlowCode {
            owner: thread_id
                .map(crate::app::DeviceFlowOwner::Turn)
                .unwrap_or(crate::app::DeviceFlowOwner::Unattributed),
            // Provider context is shown via the preceding Info message.
            provider: String::new(),
            url,
            code,
        },
        E::DeviceFlowComplete => GwEvent::DeviceFlowDone(
            thread_id
                .map(crate::app::DeviceFlowOwner::Turn)
                .unwrap_or(crate::app::DeviceFlowOwner::Unattributed),
        ),

        // ── Threads ─────────────────────────────────────────────────────
        // The TUI tab bar renders id/label/is_foreground/message_count only,
        // so the client DTO is sufficient; the icon/summary fields are unused.
        E::ThreadsUpdate {
            threads,
            foreground_id,
        } => GwEvent::ThreadsUpdate {
            threads: threads
                .into_iter()
                .map(|t| crate::action::ThreadInfo {
                    id: t.id,
                    label: t.label.unwrap_or_default(),
                    description: t.description,
                    status: if t.status.is_empty() {
                        None
                    } else {
                        Some(t.status)
                    },
                    kind_icon: None,
                    status_icon: None,
                    is_foreground: t.is_foreground,
                    message_count: t.message_count,
                    has_summary: false,
                    project_id: t.project_id,
                    working_dir: t.working_dir,
                })
                .collect(),
            foreground_id,
        },
        // The TUI has no plugin surface yet; plugin state is desktop-only for
        // now. Dropped explicitly rather than by a catch-all arm, so adding a
        // TUI plugin panel is a compile error away rather than a silent no-op.
        E::PluginsUpdate { .. } => return None,
        // Workspace file access serves editor-style UI, which the TUI does
        // not have. Explicit arms rather than a catch-all, so wiring one up
        // later is a compile error away rather than a silent no-op.
        E::WorkspaceDirListing { .. }
        | E::WorkspaceFileContent { .. }
        | E::WorkspaceWriteResult { .. } => return None,
        E::ProjectsUpdate {
            projects,
            active_id,
        } => GwEvent::ProjectsUpdate {
            projects: projects
                .into_iter()
                .map(|p| rustyclaw_core::ui::ProjectInfo {
                    id: p.id,
                    name: p.name,
                    path: p.path,
                })
                .collect(),
            active_id,
        },
        E::AgentsUpdate { agents, active_id } => GwEvent::AgentsUpdate {
            agents: agents
                .into_iter()
                .map(|a| rustyclaw_view::AgentItemData {
                    id: a.id,
                    name: a.name,
                    description: a.description,
                })
                .collect(),
            active_id,
        },
        E::AgentSwitched { agent_id, name } => GwEvent::AgentSwitched { agent_id, name },
        E::ThreadMessages {
            thread_id,
            messages,
        } => GwEvent::ThreadMessages {
            thread_id,
            messages,
        },
        E::ThreadSwitched {
            thread_id,
            context_summary,
        } => GwEvent::ThreadSwitched {
            thread_id,
            context_summary,
        },
        E::ThreadHistory {
            thread_id,
            ok,
            messages,
            error,
        } => GwEvent::ThreadHistory {
            thread_id,
            ok,
            messages,
            error,
        },

        // ── Generic messages ────────────────────────────────────────────
        E::Error { message } => GwEvent::error(message),
        E::Info { message } => GwEvent::Info(message),
        E::Warning { message } => GwEvent::warning(message),

        // DOM queries require a webview; the TUI cannot evaluate JS.
        E::DomQuery { .. } => return None,

        // ── Kernel awareness ────────────────────────────────────────────
        E::HostInfo {
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
            GwEvent::HostInfo(rustyclaw_view::HostInfoData {
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
            })
        }
        E::LoadStatus {
            load_score,
            avg_load_score,
            cpu_percent,
            memory_percent,
            summary,
        } => GwEvent::LoadStatus(rustyclaw_view::LoadStatusData {
            load_score,
            avg_load_score,
            cpu_percent,
            memory_percent,
            summary,
        }),

        // ── Secrets results ─────────────────────────────────────────────
        E::SecretsListResult { entries, .. } => {
            let secrets: Vec<rustyclaw_view::SecretInfoData> =
                entries.iter().map(Into::into).collect();
            GwEvent::ShowSecrets {
                secrets,
                agent_access: false,
                has_totp: false,
            }
        }
        E::SecretsStoreResult { ok, message } => {
            if ok {
                GwEvent::RefreshSecrets
            } else {
                GwEvent::error(format!("Failed to store secret: {}", message))
            }
        }
        E::SecretsGetResult { key, value } => {
            let display = value.as_deref().unwrap_or("(not found)");
            GwEvent::Info(format!("Secret [{}]: {}", key, display))
        }
        E::SecretsDeleteResult { ok, .. } | E::SecretsDeleteCredentialResult { ok } => {
            if ok {
                GwEvent::RefreshSecrets
            } else {
                GwEvent::error("Failed to delete credential".to_string())
            }
        }
        E::SecretsPeekResult {
            ok,
            fields,
            message,
        } => {
            if ok {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("  {}: {}", k, v))
                    .collect();
                GwEvent::Info(format!("Credential:\n{}", field_strs.join("\n")))
            } else {
                GwEvent::error(message.unwrap_or_else(|| "Failed to peek credential".to_string()))
            }
        }
        E::SecretsSetPolicyResult { ok, message } => {
            if ok {
                GwEvent::RefreshSecrets
            } else {
                GwEvent::error(message.unwrap_or_else(|| "Failed to update policy".to_string()))
            }
        }
        E::SecretsSetDisabledResult { ok } => {
            if ok {
                GwEvent::RefreshSecrets
            } else {
                GwEvent::error("Failed to update credential".to_string())
            }
        }
        E::SecretsHasTotpResult { has_totp } => GwEvent::Info(if has_totp {
            "TOTP is configured".to_string()
        } else {
            "TOTP is not configured".to_string()
        }),
        E::SecretsSetupTotpResult { ok, uri, message } => {
            if ok {
                GwEvent::Success(format!(
                    "TOTP setup complete{}",
                    uri.map(|u| format!(" — URI: {}", u)).unwrap_or_default()
                ))
            } else {
                GwEvent::error(message.unwrap_or_else(|| "TOTP setup failed".to_string()))
            }
        }
        E::SecretsVerifyTotpResult { ok } => {
            if ok {
                GwEvent::Success("TOTP verified".to_string())
            } else {
                GwEvent::error("TOTP verification failed")
            }
        }
        E::SecretsRemoveTotpResult { ok } => {
            if ok {
                GwEvent::Success("TOTP removed".to_string())
            } else {
                GwEvent::error("TOTP removal failed")
            }
        }
        E::ServiceList { services } => GwEvent::ServiceList(rustyclaw_view::ServiceListData {
            services: services.into_iter().map(Into::into).collect(),
        }),
        E::ServiceActionResult { service, .. } => GwEvent::ServiceActionResult {
            service: service.map(Into::into),
        },
        E::ServiceLogs { .. } => {
            // Logs are displayed in a separate dialog; no GwEvent needed.
            return None;
        }

        // ── Engines ──────────────────────────────────────────────────────
        E::EngineListResult { engines } => GwEvent::EngineListResult {
            engines: engines
                .into_iter()
                .map(|e| rustyclaw_view::LocalEngineData {
                    id: e.id,
                    display_name: e.display_name,
                    installed: e.installed,
                    running: e.running,
                    version: e.version,
                    endpoint: e.endpoint,
                    available_models: e.available_models,
                    loaded_models: e.loaded_models,
                    caps: rustyclaw_view::EngineCapsData {
                        can_install: e.capabilities.can_install,
                        can_start: e.capabilities.can_start,
                        can_stop: e.capabilities.can_stop,
                        can_pull: e.capabilities.can_pull,
                        can_remove: e.capabilities.can_remove,
                        can_load: e.capabilities.can_load,
                        can_unload: e.capabilities.can_unload,
                    },
                })
                .collect(),
        },
        // The TUI runs beside the vault and fetches provider models locally
        // (`CommandAction::FetchModels`), so gateway-fetched lists are not
        // surfaced here — they exist for remote clients like the desktop app.
        E::ProviderModelListResult { .. } => return None,
        E::EngineModelListResult { engine, models } => GwEvent::EngineModelListResult {
            engine: engine.clone(),
            models: models
                .into_iter()
                .map(|m| rustyclaw_view::LocalModelData {
                    engine: engine.clone(),
                    name: m.name,
                    size_bytes: m.size_bytes,
                    quantization: m.quantization,
                    context_length: m.context_length,
                    loaded: m.loaded,
                    vram_bytes: m.vram_bytes,
                    family: m.family,
                    format: m.format,
                    fits_host: m.fits_host,
                    fit_warning_msg: m.fit_warning,
                })
                .collect(),
        },
        E::EnginePullProgress {
            engine,
            model,
            percent,
            downloaded_bytes,
            total_bytes,
            status,
        } => GwEvent::EnginePullProgress {
            engine,
            model,
            percent,
            downloaded_bytes,
            total_bytes,
            status,
        },
        E::EngineActionResult {
            engine,
            model,
            ok,
            message,
        } => GwEvent::EngineActionResult {
            engine,
            model,
            ok,
            message,
        },
        E::EngineActionProgress {
            engine,
            line,
            percent: _,
        } => GwEvent::EngineActionProgress { engine, line },

        // ── Panels ───────────────────────────────────────────────────────
        E::CronListResult { jobs } => GwEvent::CronListResult {
            jobs: jobs.iter().map(Into::into).collect(),
        },
        E::CronUpsertResult { ok, message, .. } => GwEvent::PanelActionResult {
            panel: PanelKind::Cron,
            ok,
            message,
        },
        E::CronActionResult { ok, message } => GwEvent::PanelActionResult {
            panel: PanelKind::Cron,
            ok,
            message,
        },
        E::MemoryListResult { entries } => GwEvent::MemoryListResult {
            entries: entries.iter().map(Into::into).collect(),
        },
        E::MemoryUpsertResult { ok, message, .. } => GwEvent::PanelActionResult {
            panel: PanelKind::Memory,
            ok,
            message,
        },
        E::MemoryDeleteResult { ok, message } => GwEvent::PanelActionResult {
            panel: PanelKind::Memory,
            ok,
            message,
        },
        E::HistorySearchResult { entries } => GwEvent::HistorySearchResult {
            entries: entries.iter().map(Into::into).collect(),
        },
        E::McpListResult { servers } => GwEvent::McpListResult {
            servers: servers.iter().map(Into::into).collect(),
        },
        E::McpConnectResult { ok, message, .. } => GwEvent::PanelActionResult {
            panel: PanelKind::Mcp,
            ok,
            message,
        },
        E::McpDisconnectResult { ok, message } => GwEvent::PanelActionResult {
            panel: PanelKind::Mcp,
            ok,
            message,
        },
        E::ChannelStatusResult { channels } => GwEvent::ChannelStatusResult {
            channels: channels.iter().map(Into::into).collect(),
        },
        E::ChannelPairResult { ok, message, .. } => GwEvent::PanelActionResult {
            panel: PanelKind::Channels,
            ok,
            message,
        },
        E::MessengerConfigResult {
            accounts,
            routes,
            threads,
            available_kinds,
            vault_locked,
        } => GwEvent::MessengerConfigResult {
            accounts: accounts.iter().map(Into::into).collect(),
            routes: routes.iter().map(Into::into).collect(),
            threads: threads.iter().map(Into::into).collect(),
            available_kinds,
            vault_locked,
        },
        E::MessengerAccountResult {
            ok,
            errors,
            message,
            ..
        } => GwEvent::MessengerActionResult {
            ok,
            // Several validation problems come back at once; joining them
            // keeps every one visible in a one-line status area.
            message: match errors.is_empty() {
                true => message,
                false => Some(errors.join("; ")),
            },
        },
        E::MessengerRouteResult { ok, message } => GwEvent::MessengerActionResult { ok, message },
        // The TUI edits tool permissions through its local dialog.
        E::ToolConfigResult { .. } | E::ToolToggleResult { .. } => return None,
        E::UsageStatsResult {
            totals,
            per_model,
            per_session,
        } => GwEvent::UsageStatsResult {
            totals: (&totals).into(),
            per_model: per_model.iter().map(Into::into).collect(),
            per_session: per_session.iter().map(Into::into).collect(),
        },
        E::LogsResult {
            ok,
            source,
            lines,
            message,
        } => GwEvent::LogsResult {
            ok,
            source,
            lines,
            message,
        },
    };

    Some(ev)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclaw_core::gateway::{
        SecretEntryDto, ServerFrame, ServerFrameType, ServerPayload, StatusType,
    };

    /// Run a server frame through the shared parser and the TUI adapter, the
    /// same path the gateway reader takes at runtime.
    fn adapt(frame: ServerFrame) -> Option<GwEvent> {
        adapt_for(None, frame)
    }

    /// As `adapt`, for a frame the core client attributed to `thread_id`.
    fn adapt_for(thread_id: Option<u64>, frame: ServerFrame) -> Option<GwEvent> {
        GatewayEvent::from_server_frame(frame)
            .and_then(|event| gateway_event_to_gw_event(thread_id, event))
    }

    #[test]
    fn hello_frame_maps_to_connected() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Hello,
            payload: ServerPayload::Hello {
                agent: "test".into(),
                settings_dir: "/tmp".into(),
                vault_locked: false,
                provider: None,
                model: None,
            },
        };
        assert!(matches!(adapt(frame), Some(GwEvent::Connected)));
    }

    #[test]
    fn status_model_ready_maps_to_model_ready() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Status,
            payload: ServerPayload::Status {
                status: StatusType::ModelReady,
                detail: "Claude".into(),
            },
        };
        match adapt(frame) {
            Some(GwEvent::ModelReady(m)) => assert_eq!(m, "Claude"),
            other => panic!("expected ModelReady, got {other:?}"),
        }
    }

    #[test]
    fn status_vault_locked_maps_to_vault_locked() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Status,
            payload: ServerPayload::Status {
                status: StatusType::VaultLocked,
                detail: String::new(),
            },
        };
        assert!(matches!(adapt(frame), Some(GwEvent::VaultLocked)));
    }

    #[test]
    fn chunk_frame_maps_to_chunk() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Chunk,
            payload: ServerPayload::Chunk {
                delta: "Hello".into(),
            },
        };
        match adapt(frame) {
            Some(GwEvent::Chunk(_, t)) => assert_eq!(t, "Hello"),
            other => panic!("expected Chunk, got {other:?}"),
        }
    }

    #[test]
    fn tool_call_frame_maps_to_tool_call() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::ToolCall,
            payload: ServerPayload::ToolCall {
                id: "call_001".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"/tmp/test"}"#.into(),
            },
        };
        match adapt(frame) {
            Some(GwEvent::ToolCall { id, name, .. }) => {
                assert_eq!(id, "call_001");
                assert_eq!(name, "read_file");
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// A gateway flow's owner is its turn — or Unattributed from an old
    /// gateway — never Local, which is reserved for flows this client
    /// starts itself. Overloading one value for both is what let a local
    /// sign-in finishing tear down a turn's dialog.
    #[test]
    fn gateway_device_flows_are_never_owned_by_the_client() {
        fn complete() -> ServerFrame {
            ServerFrame {
                frame_type: ServerFrameType::DeviceFlowComplete,
                payload: ServerPayload::DeviceFlowComplete,
            }
        }
        match adapt_for(Some(5), complete()) {
            Some(GwEvent::DeviceFlowDone(owner)) => {
                assert_eq!(owner, crate::app::DeviceFlowOwner::Turn(5));
            }
            other => panic!("expected DeviceFlowDone, got {other:?}"),
        }
        match adapt(complete()) {
            Some(GwEvent::DeviceFlowDone(owner)) => {
                assert_eq!(owner, crate::app::DeviceFlowOwner::Unattributed);
            }
            other => panic!("expected DeviceFlowDone, got {other:?}"),
        }
    }

    #[test]
    fn tool_status_frame_maps_to_tool_status() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::ToolStatus,
            payload: ServerPayload::ToolStatus {
                tool_id: "call_001".into(),
                name: "execute_command".into(),
                elapsed_ms: 5_000,
                pid: Some(1234),
                cpu_percent: Some(42.0),
                memory_bytes: Some(1024),
                state: Some("running".into()),
                message: None,
            },
        };
        match adapt(frame) {
            Some(GwEvent::ToolStatus { id, status, .. }) => {
                assert_eq!(id, "call_001");
                assert_eq!(status.elapsed_ms, 5_000);
                assert_eq!(status.pid, Some(1234));
                assert_eq!(status.state.as_deref(), Some("running"));
                assert!(!status.is_paused());
            }
            other => panic!("expected ToolStatus, got {other:?}"),
        }
    }

    #[test]
    fn error_frame_maps_to_error() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Error,
            payload: ServerPayload::Error {
                ok: false,
                message: "Connection failed".into(),
            },
        };
        match adapt(frame) {
            Some(GwEvent::Error { summary, .. }) => assert_eq!(summary, "Connection failed"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn secrets_list_result_maps_to_show_secrets() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::SecretsListResult,
            payload: ServerPayload::SecretsListResult {
                ok: true,
                entries: vec![SecretEntryDto {
                    name: "api_key".into(),
                    label: "API Key".into(),
                    kind: "ApiKey".into(),
                    policy: "always".into(),
                    disabled: false,
                }],
            },
        };
        match adapt(frame) {
            Some(GwEvent::ShowSecrets { secrets, .. }) => assert_eq!(secrets.len(), 1),
            other => panic!("expected ShowSecrets, got {other:?}"),
        }
    }

    #[test]
    fn auth_challenge_maps_to_auth_challenge() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::AuthChallenge,
            payload: ServerPayload::AuthChallenge {
                method: "totp".into(),
            },
        };
        assert!(matches!(adapt(frame), Some(GwEvent::AuthChallenge)));
    }

    /// A chunk carries the turn it came from.
    ///
    /// `Chunk` has no thread on the wire — it belongs to its turn, and the
    /// turn announced its thread on `StreamStart`. The core client resolves
    /// that and hands the attribution down, so the TUI can tell a chunk of
    /// the answer it is showing from a chunk of one running elsewhere.
    /// Without it, two concurrent turns splice into a single message.
    #[test]
    fn a_chunk_carries_the_turn_it_came_from() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::Chunk,
            payload: ServerPayload::Chunk {
                delta: "half an answer".into(),
            },
        };
        match adapt_for(Some(9), frame) {
            Some(GwEvent::Chunk(Some(9), text)) => assert_eq!(text, "half an answer"),
            other => panic!("Expected a chunk attributed to thread 9, got {other:?}"),
        }
    }

    #[test]
    fn streaming_frames_map_to_streaming_events() {
        let start = ServerFrame {
            frame_type: ServerFrameType::StreamStart,
            payload: ServerPayload::StreamStart { thread_id: Some(4) },
        };
        assert!(matches!(adapt(start), Some(GwEvent::StreamStart(Some(4)))));

        let thinking = ServerFrame {
            frame_type: ServerFrameType::ThinkingStart,
            payload: ServerPayload::ThinkingStart,
        };
        assert!(matches!(adapt(thinking), Some(GwEvent::ThinkingStart(_))));

        let done = ServerFrame {
            frame_type: ServerFrameType::ResponseDone,
            payload: ServerPayload::ResponseDone {
                ok: true,
                thread_id: Some(4),
            },
        };
        assert!(matches!(adapt(done), Some(GwEvent::ResponseDone(Some(4)))));
    }
}
