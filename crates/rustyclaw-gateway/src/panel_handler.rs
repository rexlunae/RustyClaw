//! Handlers for the new UI panel requests (cron, memory, analytics, logs,
//! MCP, tool config, channels, approvals).
//!
//! Each handler returns a stub/empty response for now — the protocol wiring
//! is complete and clients can render the "empty state" for each panel.

use anyhow::Result;
use rustyclaw_core::gateway::TransportWriter;
use rustyclaw_core::gateway::protocol::frames::*;
use rustyclaw_core::gateway::protocol::server::send_frame;

pub async fn handle_panel_request(
    writer: &mut dyn TransportWriter,
    payload: ClientPayload,
) -> Result<()> {
    let response = match payload {
        ClientPayload::CronListRequest => ServerFrame {
            frame_type: ServerFrameType::CronListResult,
            payload: ServerPayload::CronListResult { jobs: vec![] },
        },
        ClientPayload::CronUpsertRequest { .. } => ServerFrame {
            frame_type: ServerFrameType::CronUpsertResult,
            payload: ServerPayload::CronUpsertResult {
                ok: false,
                job: None,
                message: Some("Cron system not yet initialised".into()),
            },
        },
        ClientPayload::CronActionRequest { .. } => ServerFrame {
            frame_type: ServerFrameType::CronActionResult,
            payload: ServerPayload::CronActionResult {
                ok: false,
                message: Some("Cron system not yet initialised".into()),
            },
        },
        ClientPayload::MemoryListRequest { .. } => ServerFrame {
            frame_type: ServerFrameType::MemoryListResult,
            payload: ServerPayload::MemoryListResult { entries: vec![] },
        },
        ClientPayload::MemoryUpsertRequest { .. } => ServerFrame {
            frame_type: ServerFrameType::MemoryUpsertResult,
            payload: ServerPayload::MemoryUpsertResult {
                ok: false,
                id: None,
                message: Some("Memory system not yet initialised".into()),
            },
        },
        ClientPayload::MemoryDeleteRequest { .. } => ServerFrame {
            frame_type: ServerFrameType::MemoryDeleteResult,
            payload: ServerPayload::MemoryDeleteResult {
                ok: false,
                message: Some("Memory system not yet initialised".into()),
            },
        },
        ClientPayload::HistorySearchRequest { .. } => ServerFrame {
            frame_type: ServerFrameType::HistorySearchResult,
            payload: ServerPayload::HistorySearchResult { entries: vec![] },
        },
        ClientPayload::UsageStatsRequest { period } => ServerFrame {
            frame_type: ServerFrameType::UsageStatsResult,
            payload: ServerPayload::UsageStatsResult {
                totals: UsageTotalsDto {
                    total_requests: 0,
                    total_input_tokens: 0,
                    total_output_tokens: 0,
                    total_latency_ms: 0,
                    period: period.unwrap_or_else(|| "all".into()),
                },
                per_model: vec![],
                per_session: vec![],
            },
        },
        ClientPayload::LogsRequest { source, .. } => ServerFrame {
            frame_type: ServerFrameType::LogsResult,
            payload: ServerPayload::LogsResult {
                ok: true,
                source,
                lines: vec!["(no log entries)".into()],
                message: None,
            },
        },
        ClientPayload::McpListRequest => ServerFrame {
            frame_type: ServerFrameType::McpListResult,
            payload: ServerPayload::McpListResult { servers: vec![] },
        },
        ClientPayload::McpConnectRequest { name, .. } => ServerFrame {
            frame_type: ServerFrameType::McpConnectResult,
            payload: ServerPayload::McpConnectResult {
                ok: false,
                server: None,
                message: Some(format!("MCP connect not yet implemented for '{}'", name)),
            },
        },
        ClientPayload::McpDisconnectRequest { name } => ServerFrame {
            frame_type: ServerFrameType::McpDisconnectResult,
            payload: ServerPayload::McpDisconnectResult {
                ok: false,
                message: Some(format!("MCP disconnect not yet implemented for '{}'", name)),
            },
        },
        ClientPayload::ToolConfigRequest => ServerFrame {
            frame_type: ServerFrameType::ToolConfigResult,
            payload: ServerPayload::ToolConfigResult { tools: vec![] },
        },
        ClientPayload::ToolToggleRequest { tool_name, .. } => ServerFrame {
            frame_type: ServerFrameType::ToolToggleResult,
            payload: ServerPayload::ToolToggleResult {
                ok: false,
                message: Some(format!(
                    "Tool toggle not yet implemented for '{}'",
                    tool_name
                )),
            },
        },
        ClientPayload::ChannelStatusRequest => ServerFrame {
            frame_type: ServerFrameType::ChannelStatusResult,
            payload: ServerPayload::ChannelStatusResult { channels: vec![] },
        },
        ClientPayload::ChannelPairRequest { channel, .. } => ServerFrame {
            frame_type: ServerFrameType::ChannelPairResult,
            payload: ServerPayload::ChannelPairResult {
                ok: false,
                channel: None,
                message: Some(format!(
                    "Channel pair not yet implemented for '{}'",
                    channel
                )),
            },
        },
        ClientPayload::PendingApprovalsRequest => ServerFrame {
            frame_type: ServerFrameType::PendingApprovalsResult,
            payload: ServerPayload::PendingApprovalsResult { approvals: vec![] },
        },
        ClientPayload::ApprovalsBatchAction { .. } => ServerFrame {
            frame_type: ServerFrameType::ApprovalsBatchResult,
            payload: ServerPayload::ApprovalsBatchResult {
                ok: false,
                message: Some("Approvals batch not yet implemented".into()),
            },
        },
        ClientPayload::VoiceStart { .. }
        | ClientPayload::VoiceStop
        | ClientPayload::VoiceAudioChunk { .. } => ServerFrame {
            frame_type: ServerFrameType::VoiceStateUpdate,
            payload: ServerPayload::VoiceStateUpdate {
                state: "idle".into(),
            },
        },
        ClientPayload::PreviewRequest { path } => ServerFrame {
            frame_type: ServerFrameType::PreviewResult,
            payload: ServerPayload::PreviewResult {
                path,
                kind: "none".into(),
                content: String::new(),
                error: Some("Preview not yet implemented".into()),
            },
        },
        ClientPayload::PreviewFollowToggle { path, .. } => ServerFrame {
            frame_type: ServerFrameType::PreviewResult,
            payload: ServerPayload::PreviewResult {
                path,
                kind: "none".into(),
                content: String::new(),
                error: Some("File-follow not yet implemented".into()),
            },
        },
        _ => return Ok(()),
    };

    send_frame(writer, &response).await
}
