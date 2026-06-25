//! Client-side protocol adapter.
//!
//! Wire-frame parsing is shared in `rustyclaw_core::gateway`
//! ([`GatewayEvent::from_server_frame`]); the core [`GatewayClient`] produces
//! [`GatewayEvent`]s, and this module adapts those into the TUI's UI-level
//! [`GwEvent`]s (dialog prompts, render updates, status messages). It is the
//! single translation between the shared event model and the TUI.
//!
//! [`GatewayClient`]: rustyclaw_core::gateway::GatewayClient

use crate::app::GwEvent;
use rustyclaw_core::gateway::{GatewayEvent, SecretEntryDto};

/// Adapt a shared gateway event into a TUI UI event.
///
/// Returns `None` for events the TUI does not surface (e.g. DOM queries, which
/// require a webview the TUI does not have).
pub(crate) fn gateway_event_to_gw_event(event: GatewayEvent) -> Option<GwEvent> {
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
        E::StreamStart => GwEvent::StreamStart,
        E::ThinkingStart => GwEvent::ThinkingStart,
        E::ThinkingDelta => GwEvent::ThinkingDelta,
        E::ThinkingEnd => GwEvent::ThinkingEnd,
        E::Chunk { delta } => GwEvent::Chunk(delta),
        E::ResponseDone => GwEvent::ResponseDone,

        // ── Tool calls ──────────────────────────────────────────────────
        E::ToolCall {
            id,
            name,
            arguments,
        } => GwEvent::ToolCall {
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
            id,
            name,
            result,
            is_error,
        },
        E::ToolApprovalRequest {
            id,
            name,
            arguments,
        } => GwEvent::ToolApprovalRequest {
            id,
            name,
            arguments,
        },

        // ── Interactive prompts ─────────────────────────────────────────
        E::UserPromptRequest { prompt, .. } => GwEvent::UserPromptRequest(prompt),
        E::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        } => GwEvent::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        },
        E::DeviceFlowStart { url, code, .. } => GwEvent::DeviceFlowCode {
            // Provider context is shown via the preceding Info message.
            provider: String::new(),
            url,
            code,
        },
        E::DeviceFlowComplete => GwEvent::DeviceFlowDone,

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
                })
                .collect(),
            foreground_id,
        },
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
            let secrets: Vec<rustyclaw_view::SecretInfoData> = entries
                .into_iter()
                .map(|e| {
                    // `SecretEntryInfo` and the wire `SecretEntryDto` are
                    // field-for-field identical.
                    rustyclaw_view::SecretInfoData::from_dto(SecretEntryDto {
                        name: e.name,
                        label: e.label,
                        kind: e.kind,
                        policy: e.policy,
                        disabled: e.disabled,
                    })
                })
                .collect();
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
        E::ServiceList { services } => {
            let dto_to_info =
                |d: rustyclaw_core::gateway::protocol::frames::ServiceInfoDto| {
                    rustyclaw_view::ServiceInfoData {
                        name: d.name,
                        service_type: d.service_type,
                        status: d.status,
                        pid: d.pid,
                        uptime_secs: d.uptime_secs,
                        restart_count: d.restart_count,
                        exit_code: d.exit_code,
                        health_ok: d.health_ok,
                        mcp_tools: d.mcp_tools,
                    }
                };
            GwEvent::ServiceList(rustyclaw_view::ServiceListData {
                services: services.into_iter().map(dto_to_info).collect(),
            })
        }
        E::ServiceActionResult {
            service, ..
        } => {
            let info = service.map(|d| rustyclaw_view::ServiceInfoData {
                name: d.name,
                service_type: d.service_type,
                status: d.status,
                pid: d.pid,
                uptime_secs: d.uptime_secs,
                restart_count: d.restart_count,
                exit_code: d.exit_code,
                health_ok: d.health_ok,
                mcp_tools: d.mcp_tools,
            });
            GwEvent::ServiceActionResult { service: info }
        }
        E::ServiceLogs { .. } => {
            // Logs are displayed in a separate dialog; no GwEvent needed.
            return None;
        }
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
        GatewayEvent::from_server_frame(frame).and_then(gateway_event_to_gw_event)
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
            Some(GwEvent::Chunk(t)) => assert_eq!(t, "Hello"),
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

    #[test]
    fn streaming_frames_map_to_streaming_events() {
        let start = ServerFrame {
            frame_type: ServerFrameType::StreamStart,
            payload: ServerPayload::StreamStart,
        };
        assert!(matches!(adapt(start), Some(GwEvent::StreamStart)));

        let thinking = ServerFrame {
            frame_type: ServerFrameType::ThinkingStart,
            payload: ServerPayload::ThinkingStart,
        };
        assert!(matches!(adapt(thinking), Some(GwEvent::ThinkingStart)));

        let done = ServerFrame {
            frame_type: ServerFrameType::ResponseDone,
            payload: ServerPayload::ResponseDone { ok: true },
        };
        assert!(matches!(adapt(done), Some(GwEvent::ResponseDone)));
    }
}
