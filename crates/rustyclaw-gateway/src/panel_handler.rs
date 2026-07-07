//! Handlers for the UI panel requests (cron, memory, MCP, tool config,
//! channels, approvals, …).
//!
//! Wired panels operate on the same backends the AI tools use:
//!
//! * **Cron** — the persistent [`CronStore`] under `<workspace>/.cron`.
//! * **Memory** — `MEMORY.md` / `HISTORY.md` via [`MemoryConsolidation`].
//! * **MCP** — the shared [`McpManager`] registered in `runtime_ctx` at
//!   startup (requires the `mcp` feature).
//! * **Tool config** — the tool registry plus `config.tool_permissions`.
//! * **Channels** — the `[[messengers]]` config entries.
//!
//! Panels whose backing subsystem does not exist yet (usage analytics,
//! logs, approvals queue, voice, preview) still return their empty-state
//! stub responses.

use anyhow::Result;
use rustyclaw_core::config::Config;
use rustyclaw_core::cron::{CronJob, CronJobPatch, CronStore, Payload, Schedule, SessionTarget};
use rustyclaw_core::gateway::TransportWriter;
use rustyclaw_core::gateway::protocol::frames::*;
use rustyclaw_core::gateway::protocol::server::send_frame;
use rustyclaw_core::memory_consolidation::{ConsolidationConfig, MemoryConsolidation};

pub async fn handle_panel_request(
    writer: &mut dyn TransportWriter,
    payload: ClientPayload,
    config: &mut Config,
) -> Result<()> {
    let response = match payload {
        // ── Cron ─────────────────────────────────────────────────────────
        ClientPayload::CronListRequest => cron_list(config),
        ClientPayload::CronUpsertRequest {
            id,
            name,
            expr,
            payload,
            paused,
        } => cron_upsert(config, id, name, expr, payload, paused),
        ClientPayload::CronActionRequest { id, action } => cron_action(config, id, action),

        // ── Memory ───────────────────────────────────────────────────────
        ClientPayload::MemoryListRequest { query, limit } => memory_list(config, query, limit),
        ClientPayload::MemoryUpsertRequest {
            id,
            content,
            category,
        } => memory_upsert(config, id, content, category),
        ClientPayload::MemoryDeleteRequest { id } => memory_delete(config, id),
        ClientPayload::HistorySearchRequest { query, limit } => {
            history_search(config, query, limit)
        }

        // ── MCP ──────────────────────────────────────────────────────────
        ClientPayload::McpListRequest => mcp_list(config).await,
        ClientPayload::McpConnectRequest {
            name,
            command,
            url,
            env,
        } => mcp_connect(config, name, command, url, env).await,
        ClientPayload::McpDisconnectRequest { name } => mcp_disconnect(name).await,

        // ── Tool config ──────────────────────────────────────────────────
        ClientPayload::ToolConfigRequest => tool_config_list(config),
        ClientPayload::ToolToggleRequest { tool_name, enabled } => {
            tool_toggle(config, tool_name, enabled)
        }

        // ── Channels ─────────────────────────────────────────────────────
        ClientPayload::ChannelStatusRequest => channel_status(config),
        ClientPayload::ChannelPairRequest { channel, action } => {
            channel_pair(config, channel, action)
        }

        // ── Analytics / logs (from the stats observer) ───────────────────
        ClientPayload::UsageStatsRequest { period } => usage_stats(period),
        ClientPayload::LogsRequest { source, tail, .. } => logs(source, tail).await,

        // ── Still stubbed: no backing subsystem yet ──────────────────────
        // Approvals need a queue (the Ask flow is a blocking per-request
        // channel); voice and preview are unbuilt end to end.
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

// ═════════════════════════════════════════════════════════════════════════
// Analytics / logs
// ═════════════════════════════════════════════════════════════════════════

/// Convert the panel's period string into a "records since" timestamp.
fn period_start_ms(period: &str) -> Option<u64> {
    let day_ms: u64 = 24 * 60 * 60 * 1000;
    let window = match period {
        "day" => day_ms,
        "week" => 7 * day_ms,
        "month" => 30 * day_ms,
        _ => return None, // "all" and anything unrecognised
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Some(now.saturating_sub(window))
}

fn usage_stats(period: Option<String>) -> ServerFrame {
    let period = period.unwrap_or_else(|| "all".into());
    let usage = rustyclaw_core::runtime_ctx::get_stats_observer()
        .map(|stats| stats.usage(period_start_ms(&period)))
        .unwrap_or_default();

    ServerFrame {
        frame_type: ServerFrameType::UsageStatsResult,
        payload: ServerPayload::UsageStatsResult {
            totals: UsageTotalsDto {
                total_requests: usage.total_requests,
                total_input_tokens: usage.total_input_tokens,
                total_output_tokens: usage.total_output_tokens,
                total_latency_ms: usage.total_latency_ms,
                period,
            },
            per_model: usage
                .per_model
                .into_iter()
                .map(|m| ModelUsageDto {
                    provider: m.provider,
                    model: m.model,
                    requests: m.requests,
                    input_tokens: m.input_tokens,
                    output_tokens: m.output_tokens,
                    avg_latency_ms: m.avg_latency_ms,
                })
                .collect(),
            // Observer events don't carry session ids yet.
            per_session: vec![],
        },
    }
}

async fn logs(source: String, tail: Option<usize>) -> ServerFrame {
    let tail = tail.unwrap_or(200);
    let (ok, lines, message) = match source.as_str() {
        // The gateway/agent share one telemetry ring (LLM calls, tool
        // calls, channel traffic, errors).
        "gateway" | "agent" => match rustyclaw_core::runtime_ctx::get_stats_observer() {
            Some(stats) => {
                let lines = stats.recent_logs(tail);
                if lines.is_empty() {
                    (true, vec!["(no log entries yet)".into()], None)
                } else {
                    (true, lines, None)
                }
            }
            None => (
                false,
                Vec::new(),
                Some("Telemetry observer not initialised".to_string()),
            ),
        },
        "cron" => (
            false,
            Vec::new(),
            Some("Cron has no runtime yet, so there are no run logs".to_string()),
        ),
        // Anything else is a managed-service name.
        service => match rustyclaw_core::runtime_ctx::get_service_manager() {
            Some(mgr) => {
                let mgr = mgr.read().await;
                match mgr.logs(service, Some(tail)) {
                    Ok(lines) => (true, lines, None),
                    Err(e) => (false, Vec::new(), Some(e)),
                }
            }
            None => (
                false,
                Vec::new(),
                Some("Service manager not initialised".to_string()),
            ),
        },
    };

    ServerFrame {
        frame_type: ServerFrameType::LogsResult,
        payload: ServerPayload::LogsResult {
            ok,
            source,
            lines,
            message,
        },
    }
}

// ═════════════════════════════════════════════════════════════════════════
// Cron
// ═════════════════════════════════════════════════════════════════════════

fn open_cron_store(config: &Config) -> Result<CronStore, String> {
    let cron_dir = config.workspace_dir().join(".cron");
    CronStore::new(&cron_dir).map_err(|e| e.to_string())
}

fn ms_to_rfc3339(ms: u64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms as i64).map(|t| t.to_rfc3339())
}

/// Render a schedule in the panel's single-string `expr` form. The inverse
/// of [`parse_schedule`].
fn render_schedule(schedule: &Schedule) -> String {
    match schedule {
        Schedule::At { at } => format!("at {}", at),
        Schedule::Every { every_ms, .. } => format!("every {}ms", every_ms),
        Schedule::Cron { expr, tz } => match tz {
            Some(tz) => format!("{} ({})", expr, tz),
            None => expr.clone(),
        },
    }
}

/// Parse the panel's `expr` string into a schedule: `at <ISO-8601>`,
/// `every <N>(ms|s|m|h)`, or a 5-field cron expression.
fn parse_schedule(expr: &str) -> Result<Schedule, String> {
    let expr = expr.trim();
    if let Some(at) = expr.strip_prefix("at ") {
        return Ok(Schedule::At {
            at: at.trim().to_string(),
        });
    }
    if let Some(every) = expr.strip_prefix("every ") {
        let every = every.trim();
        let (num, unit) = every
            .find(|c: char| !c.is_ascii_digit())
            .map(|i| every.split_at(i))
            .unwrap_or((every, "ms"));
        let n: u64 = num
            .parse()
            .map_err(|_| format!("Invalid interval: '{}'", every))?;
        let every_ms = match unit.trim() {
            "ms" | "" => n,
            "s" | "sec" | "secs" => n * 1_000,
            "m" | "min" | "mins" => n * 60_000,
            "h" | "hr" | "hrs" => n * 3_600_000,
            other => return Err(format!("Unknown interval unit: '{}'", other)),
        };
        if every_ms == 0 {
            return Err("Interval must be greater than zero".into());
        }
        return Ok(Schedule::Every {
            every_ms,
            anchor_ms: None,
        });
    }
    if expr.split_whitespace().count() == 5 {
        return Ok(Schedule::Cron {
            expr: expr.to_string(),
            tz: None,
        });
    }
    Err(format!(
        "Unrecognised schedule '{}' — use 'at <ISO-8601>', 'every <N>[ms|s|m|h]', or a 5-field cron expression",
        expr
    ))
}

fn job_to_dto(store: &CronStore, job: &CronJob) -> CronJobDto {
    let runs = store.get_runs(&job.job_id, 500).unwrap_or_default();
    let last = runs.first();
    CronJobDto {
        id: job.job_id.clone(),
        name: job.name.clone().unwrap_or_else(|| "(unnamed)".into()),
        expr: render_schedule(&job.schedule),
        payload: match &job.payload {
            Payload::SystemEvent { text } => text.clone(),
            Payload::AgentTurn { message, .. } => message.clone(),
        },
        paused: !job.enabled,
        next_run: job.next_run_ms.and_then(ms_to_rfc3339),
        last_run: job.last_run_ms.and_then(ms_to_rfc3339),
        last_status: last.map(|r| format!("{:?}", r.status).to_lowercase()),
        run_count: runs.len() as u64,
    }
}

fn cron_list(config: &Config) -> ServerFrame {
    let jobs = match open_cron_store(config) {
        Ok(store) => {
            let mut dtos: Vec<CronJobDto> = store
                .list(true)
                .iter()
                .map(|j| job_to_dto(&store, j))
                .collect();
            dtos.sort_by(|a, b| a.name.cmp(&b.name));
            dtos
        }
        Err(_) => vec![],
    };
    ServerFrame {
        frame_type: ServerFrameType::CronListResult,
        payload: ServerPayload::CronListResult { jobs },
    }
}

fn cron_upsert(
    config: &Config,
    id: Option<String>,
    name: String,
    expr: String,
    payload: String,
    paused: bool,
) -> ServerFrame {
    let result = (|| -> Result<CronJobDto, String> {
        let mut store = open_cron_store(config)?;
        let schedule = parse_schedule(&expr)?;
        let job_id = match id {
            Some(id) => {
                store
                    .update(
                        &id,
                        CronJobPatch {
                            name: Some(name),
                            enabled: Some(!paused),
                            schedule: Some(schedule),
                            payload: Some(Payload::SystemEvent { text: payload }),
                            delivery: None,
                        },
                    )
                    .map_err(|e| e.to_string())?;
                id
            }
            None => {
                let mut job = CronJob::new(
                    Some(name),
                    schedule,
                    SessionTarget::Main,
                    Payload::SystemEvent { text: payload },
                );
                job.enabled = !paused;
                store.add(job).map_err(|e| e.to_string())?
            }
        };
        let job = store
            .get(&job_id)
            .ok_or_else(|| format!("Job not found after save: {}", job_id))?;
        Ok(job_to_dto(&store, job))
    })();

    let (ok, job, message) = match result {
        Ok(dto) => (true, Some(dto), None),
        Err(e) => (false, None, Some(e)),
    };
    ServerFrame {
        frame_type: ServerFrameType::CronUpsertResult,
        payload: ServerPayload::CronUpsertResult { ok, job, message },
    }
}

fn cron_action(config: &Config, id: String, action: CronActionKind) -> ServerFrame {
    let result = (|| -> Result<Option<String>, String> {
        let mut store = open_cron_store(config)?;
        match action {
            CronActionKind::Pause => {
                store
                    .update(
                        &id,
                        CronJobPatch {
                            enabled: Some(false),
                            ..Default::default()
                        },
                    )
                    .map_err(|e| e.to_string())?;
                Ok(None)
            }
            CronActionKind::Resume => {
                store
                    .update(
                        &id,
                        CronJobPatch {
                            enabled: Some(true),
                            ..Default::default()
                        },
                    )
                    .map_err(|e| e.to_string())?;
                Ok(None)
            }
            CronActionKind::Remove => {
                store.remove(&id).map_err(|e| e.to_string())?;
                Ok(None)
            }
            CronActionKind::Run => {
                // No scheduler runtime exists yet to execute a job on demand.
                Err("Run-now is not available: the cron runtime is not yet wired".into())
            }
        }
    })();

    let (ok, message) = match result {
        Ok(msg) => (true, msg),
        Err(e) => (false, Some(e)),
    };
    ServerFrame {
        frame_type: ServerFrameType::CronActionResult,
        payload: ServerPayload::CronActionResult { ok, message },
    }
}

// ═════════════════════════════════════════════════════════════════════════
// Memory
// ═════════════════════════════════════════════════════════════════════════
//
// MEMORY.md is a curated markdown document. The panel views it as a list of
// entries: every `- ` / `* ` bullet is an entry, and `## ` headings set the
// category of the bullets that follow. Entry ids are positional (`L<line>`,
// 1-based) and refreshed by every list — clients act on a fresh snapshot.

struct MemoryEntryRef {
    line_idx: usize,
    content: String,
    category: Option<String>,
}

fn memory_backend(config: &Config) -> (MemoryConsolidation, std::path::PathBuf) {
    (
        MemoryConsolidation::new(ConsolidationConfig::default()),
        config.workspace_dir(),
    )
}

fn parse_memory_entries(text: &str) -> Vec<MemoryEntryRef> {
    let mut entries = Vec::new();
    let mut category: Option<String> = None;
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("## ") {
            category = Some(heading.trim().to_string());
        } else if let Some(bullet) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            entries.push(MemoryEntryRef {
                line_idx: idx,
                content: bullet.trim().to_string(),
                category: category.clone(),
            });
        }
    }
    entries
}

fn memory_list(config: &Config, query: Option<String>, limit: Option<usize>) -> ServerFrame {
    let (mem, workspace) = memory_backend(config);
    let text = mem.read_memory(&workspace).unwrap_or_default();
    let query_lower = query.as_deref().unwrap_or("").to_lowercase();
    let limit = limit.unwrap_or(200);

    let entries: Vec<MemoryEntryDto> = parse_memory_entries(&text)
        .into_iter()
        .filter(|e| {
            query_lower.is_empty()
                || e.content.to_lowercase().contains(&query_lower)
                || e.category
                    .as_deref()
                    .is_some_and(|c| c.to_lowercase().contains(&query_lower))
        })
        .take(limit)
        .map(|e| MemoryEntryDto {
            id: format!("L{}", e.line_idx + 1),
            content: e.content,
            category: e.category,
            created_at: None,
            updated_at: None,
            score: None,
        })
        .collect();

    ServerFrame {
        frame_type: ServerFrameType::MemoryListResult,
        payload: ServerPayload::MemoryListResult { entries },
    }
}

/// Resolve an `L<line>` id to a 0-based line index.
fn parse_line_id(id: &str) -> Result<usize, String> {
    id.strip_prefix('L')
        .and_then(|n| n.parse::<usize>().ok())
        .and_then(|n| n.checked_sub(1))
        .ok_or_else(|| format!("Invalid memory entry id: '{}'", id))
}

fn memory_upsert(
    config: &Config,
    id: Option<String>,
    content: String,
    category: Option<String>,
) -> ServerFrame {
    let (mem, workspace) = memory_backend(config);
    let result = (|| -> Result<String, String> {
        let text = mem.read_memory(&workspace).map_err(|e| e.to_string())?;
        let mut lines: Vec<String> = text.lines().map(String::from).collect();
        let content = content.trim();
        if content.is_empty() {
            return Err("Memory entry content cannot be empty".into());
        }

        let entry_line = format!("- {}", content);
        let new_id = match id {
            Some(id) => {
                // Replace an existing bullet in place, preserving indentation.
                let idx = parse_line_id(&id)?;
                let line = lines
                    .get_mut(idx)
                    .ok_or_else(|| format!("Memory entry not found: '{}'", id))?;
                let trimmed = line.trim_start();
                if !trimmed.starts_with("- ") && !trimmed.starts_with("* ") {
                    return Err(format!("Line {} is not a memory entry", idx + 1));
                }
                let indent = &line[..line.len() - trimmed.len()];
                *line = format!("{}- {}", indent, content);
                id
            }
            None => {
                // Append under the category heading (created if missing),
                // or at the end of the document when no category is given.
                let insert_at = match &category {
                    Some(cat) => {
                        let heading = format!("## {}", cat.trim());
                        let heading_idx = lines.iter().position(|l| l.trim() == heading.as_str());
                        match heading_idx {
                            Some(h) => {
                                // Insert after the last line of this section.
                                let mut end = h + 1;
                                while end < lines.len()
                                    && !lines[end].trim_start().starts_with("## ")
                                {
                                    end += 1;
                                }
                                // Back up over trailing blank lines.
                                while end > h + 1 && lines[end - 1].trim().is_empty() {
                                    end -= 1;
                                }
                                end
                            }
                            None => {
                                if !lines.is_empty()
                                    && !lines.last().is_some_and(|l| l.trim().is_empty())
                                {
                                    lines.push(String::new());
                                }
                                lines.push(heading);
                                lines.len()
                            }
                        }
                    }
                    None => lines.len(),
                };
                lines.insert(insert_at, entry_line);
                format!("L{}", insert_at + 1)
            }
        };

        let mut new_text = lines.join("\n");
        new_text.push('\n');
        mem.update_memory(&workspace, &new_text)
            .map_err(|e| e.to_string())?;
        Ok(new_id)
    })();

    let (ok, id, message) = match result {
        Ok(id) => (true, Some(id), None),
        Err(e) => (false, None, Some(e)),
    };
    ServerFrame {
        frame_type: ServerFrameType::MemoryUpsertResult,
        payload: ServerPayload::MemoryUpsertResult { ok, id, message },
    }
}

fn memory_delete(config: &Config, id: String) -> ServerFrame {
    let (mem, workspace) = memory_backend(config);
    let result = (|| -> Result<(), String> {
        let text = mem.read_memory(&workspace).map_err(|e| e.to_string())?;
        let mut lines: Vec<String> = text.lines().map(String::from).collect();
        let idx = parse_line_id(&id)?;
        let line = lines
            .get(idx)
            .ok_or_else(|| format!("Memory entry not found: '{}'", id))?;
        let trimmed = line.trim_start();
        if !trimmed.starts_with("- ") && !trimmed.starts_with("* ") {
            return Err(format!("Line {} is not a memory entry", idx + 1));
        }
        lines.remove(idx);
        let mut new_text = lines.join("\n");
        if !new_text.is_empty() {
            new_text.push('\n');
        }
        mem.update_memory(&workspace, &new_text)
            .map_err(|e| e.to_string())?;
        Ok(())
    })();

    let (ok, message) = match result {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e)),
    };
    ServerFrame {
        frame_type: ServerFrameType::MemoryDeleteResult,
        payload: ServerPayload::MemoryDeleteResult { ok, message },
    }
}

fn history_search(config: &Config, query: String, limit: Option<usize>) -> ServerFrame {
    let (mem, workspace) = memory_backend(config);
    let entries = mem
        .search_history(&workspace, &query, limit.unwrap_or(50))
        .unwrap_or_default()
        .into_iter()
        .map(|e| HistoryEntryDto {
            timestamp: e.timestamp,
            role: "history".into(),
            content: e.text,
            thread_id: None,
        })
        .collect();

    ServerFrame {
        frame_type: ServerFrameType::HistorySearchResult,
        payload: ServerPayload::HistorySearchResult { entries },
    }
}

// ═════════════════════════════════════════════════════════════════════════
// MCP
// ═════════════════════════════════════════════════════════════════════════

#[cfg(feature = "mcp")]
async fn mcp_list(config: &Config) -> ServerFrame {
    let mgr = rustyclaw_core::runtime_ctx::get_mcp_manager();
    let mut servers: Vec<McpServerDto> = Vec::new();

    // Live status for connected servers.
    let mut connected: std::collections::HashMap<String, Vec<String>> = Default::default();
    if let Some(mgr) = &mgr {
        let mgr = mgr.lock().await;
        for tool in mgr.list_all_tools().await {
            connected
                .entry(tool.server_name.clone())
                .or_default()
                .push(tool.prefixed_name());
        }
        for name in mgr.list_servers().await {
            connected.entry(name).or_default();
        }
    }

    // Configured servers, live or not.
    for (name, cfg) in &config.mcp.servers {
        let tools = connected.remove(name);
        let status = match (&tools, cfg.enabled) {
            (Some(_), _) => "connected",
            (None, true) => "disconnected",
            (None, false) => "disabled",
        };
        servers.push(McpServerDto {
            name: name.clone(),
            status: status.into(),
            command: Some(
                std::iter::once(cfg.command.clone())
                    .chain(cfg.args.iter().cloned())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            url: None,
            tools: tools.unwrap_or_default(),
            health_ok: None,
        });
    }

    // Ad-hoc connections that aren't in the config.
    for (name, tools) in connected {
        servers.push(McpServerDto {
            name,
            status: "connected".into(),
            command: None,
            url: None,
            tools,
            health_ok: None,
        });
    }

    servers.sort_by(|a, b| a.name.cmp(&b.name));
    ServerFrame {
        frame_type: ServerFrameType::McpListResult,
        payload: ServerPayload::McpListResult { servers },
    }
}

#[cfg(feature = "mcp")]
async fn mcp_connect(
    config: &mut Config,
    name: String,
    command: Option<String>,
    url: Option<String>,
    env: Vec<(String, String)>,
) -> ServerFrame {
    use rustyclaw_core::mcp::McpServerConfig;

    let result: Result<McpServerDto, String> = async {
        if url.is_some() {
            return Err(
                "URL-based MCP transports are not yet supported — use a stdio command".into(),
            );
        }
        let mgr =
            rustyclaw_core::runtime_ctx::get_mcp_manager().ok_or("MCP manager not initialised")?;

        // An explicit command defines (and persists) the server; otherwise
        // the name must refer to a configured server.
        let server_cfg = match command {
            Some(cmdline) => {
                let mut parts = cmdline.split_whitespace().map(String::from);
                let command = parts.next().ok_or("Empty MCP server command")?;
                let cfg = McpServerConfig {
                    command,
                    args: parts.collect(),
                    env: env.into_iter().collect(),
                    ..Default::default()
                };
                config.mcp.servers.insert(name.clone(), cfg.clone());
                if let Err(e) = config.save(None) {
                    tracing::warn!(error = %e, "Failed to persist MCP server config");
                }
                cfg
            }
            None => config
                .mcp
                .servers
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("Unknown MCP server: '{}'", name))?,
        };

        let mgr = mgr.lock().await;
        mgr.connect(&name, &server_cfg)
            .await
            .map_err(|e| e.to_string())?;
        let tools = mgr
            .list_tools(&name)
            .await
            .map(|ts| ts.iter().map(|t| t.prefixed_name()).collect())
            .unwrap_or_default();

        Ok(McpServerDto {
            name: name.clone(),
            status: "connected".into(),
            command: Some(
                std::iter::once(server_cfg.command.clone())
                    .chain(server_cfg.args.iter().cloned())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            url: None,
            tools,
            health_ok: Some(true),
        })
    }
    .await;

    let (ok, server, message) = match result {
        Ok(dto) => (true, Some(dto), None),
        Err(e) => (false, None, Some(e)),
    };
    ServerFrame {
        frame_type: ServerFrameType::McpConnectResult,
        payload: ServerPayload::McpConnectResult {
            ok,
            server,
            message,
        },
    }
}

#[cfg(feature = "mcp")]
async fn mcp_disconnect(name: String) -> ServerFrame {
    let result: Result<(), String> = async {
        let mgr =
            rustyclaw_core::runtime_ctx::get_mcp_manager().ok_or("MCP manager not initialised")?;
        let mgr = mgr.lock().await;
        mgr.disconnect(&name).await.map_err(|e| e.to_string())
    }
    .await;

    let (ok, message) = match result {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e)),
    };
    ServerFrame {
        frame_type: ServerFrameType::McpDisconnectResult,
        payload: ServerPayload::McpDisconnectResult { ok, message },
    }
}

#[cfg(not(feature = "mcp"))]
async fn mcp_list(_config: &Config) -> ServerFrame {
    ServerFrame {
        frame_type: ServerFrameType::McpListResult,
        payload: ServerPayload::McpListResult { servers: vec![] },
    }
}

#[cfg(not(feature = "mcp"))]
async fn mcp_connect(
    _config: &mut Config,
    name: String,
    _command: Option<String>,
    _url: Option<String>,
    _env: Vec<(String, String)>,
) -> ServerFrame {
    ServerFrame {
        frame_type: ServerFrameType::McpConnectResult,
        payload: ServerPayload::McpConnectResult {
            ok: false,
            server: None,
            message: Some(format!(
                "MCP support is not compiled in (server '{}') — rebuild with --features mcp",
                name
            )),
        },
    }
}

#[cfg(not(feature = "mcp"))]
async fn mcp_disconnect(name: String) -> ServerFrame {
    ServerFrame {
        frame_type: ServerFrameType::McpDisconnectResult,
        payload: ServerPayload::McpDisconnectResult {
            ok: false,
            message: Some(format!(
                "MCP support is not compiled in (server '{}') — rebuild with --features mcp",
                name
            )),
        },
    }
}

// ═════════════════════════════════════════════════════════════════════════
// Tool config
// ═════════════════════════════════════════════════════════════════════════

/// Rough grouping of registry tools for the panel.
fn tool_category(name: &str) -> &'static str {
    match name {
        "read_file" | "write_file" | "edit_file" | "list_directory" | "search_files"
        | "find_files" | "apply_patch" => "files",
        "execute_command" | "process" => "runtime",
        "web_fetch" | "web_search" | "web_extract" => "web",
        "memory_search" | "memory_get" | "save_memory" | "search_history" | "add_memory" => {
            "memory"
        }
        "cron" => "scheduling",
        n if n.starts_with("sessions_") || n == "session_status" || n == "agents_list" => {
            "sessions"
        }
        n if n.starts_with("secrets_") => "secrets",
        "gateway" => "gateway",
        "message" | "tts" => "messaging",
        "image" | "image_generate" => "media",
        "nodes" | "canvas" => "devices",
        "browser" => "browser",
        n if n.starts_with("skill_") => "skills",
        n if n.starts_with("mcp_") => "mcp",
        n if n.starts_with("task_") => "tasks",
        "thread_describe" | "set_thread_caption" => "threads",
        n if n.starts_with("model_") => "models",
        "host_info" | "load_status" => "system",
        n if n.starts_with("service_") => "services",
        "disk_usage" | "classify_files" | "system_monitor" | "battery_health" | "app_index"
        | "cloud_browse" | "browser_cache" | "screenshot" | "clipboard" | "audit_sensitive"
        | "secure_delete" | "summarize_file" => "system",
        "pkg_manage" | "net_info" | "net_scan" | "service_manage" | "user_manage" | "firewall" => {
            "sysadmin"
        }
        "ollama_manage" | "exo_manage" | "agent_setup" => "engines",
        "ast_grep_manage" | "uv_manage" | "npm_manage" => "code",
        "pdf" => "documents",
        n if n.starts_with("swarm_") => "swarm",
        "todo" => "planning",
        "skill_curator" => "skills",
        "ask_user" | "client_dom_query" => "interactive",
        _ => "other",
    }
}

fn tool_config_list(config: &Config) -> ServerFrame {
    use rustyclaw_core::tools::{ToolPermission, all_tools, tool_summary};

    let mut tools: Vec<ToolConfigDto> = all_tools()
        .iter()
        .map(|def| {
            let permission = config
                .tool_permissions
                .get(def.name)
                .cloned()
                .unwrap_or_default();
            let summary = tool_summary(def.name);
            ToolConfigDto {
                name: def.name.to_string(),
                category: tool_category(def.name).to_string(),
                enabled: !matches!(permission, ToolPermission::Deny),
                policy: permission.to_string(),
                description: if summary == "Unknown tool" {
                    def.description
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_string()
                } else {
                    summary.to_string()
                },
            }
        })
        .collect();
    tools.sort_by(|a, b| {
        (a.category.clone(), a.name.clone()).cmp(&(b.category.clone(), b.name.clone()))
    });

    ServerFrame {
        frame_type: ServerFrameType::ToolConfigResult,
        payload: ServerPayload::ToolConfigResult { tools },
    }
}

fn tool_toggle(config: &mut Config, tool_name: String, enabled: bool) -> ServerFrame {
    use rustyclaw_core::tools::{ToolPermission, all_tools};

    let result = (|| -> Result<(), String> {
        if !all_tools().iter().any(|def| def.name == tool_name) {
            return Err(format!("Unknown tool: '{}'", tool_name));
        }
        let permission = if enabled {
            ToolPermission::Allow
        } else {
            ToolPermission::Deny
        };
        config.tool_permissions.insert(tool_name, permission);
        config.save(None).map_err(|e| e.to_string())
    })();

    let (ok, message) = match result {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e)),
    };
    ServerFrame {
        frame_type: ServerFrameType::ToolToggleResult,
        payload: ServerPayload::ToolToggleResult { ok, message },
    }
}

// ═════════════════════════════════════════════════════════════════════════
// Channels
// ═════════════════════════════════════════════════════════════════════════

fn messenger_display_name(m: &rustyclaw_core::config::MessengerConfig) -> String {
    if m.name.is_empty() {
        m.messenger_type.clone()
    } else {
        m.name.clone()
    }
}

/// Whether a messenger entry has the credentials its backend needs.
fn messenger_has_credentials(m: &rustyclaw_core::config::MessengerConfig) -> bool {
    m.token.is_some()
        || m.webhook_url.is_some()
        || m.access_token.is_some()
        || m.password.is_some()
        || m.phone.is_some()
        || m.config_path.is_some()
}

fn messenger_to_dto(m: &rustyclaw_core::config::MessengerConfig) -> ChannelStatusDto {
    ChannelStatusDto {
        name: messenger_display_name(m),
        channel_type: m.messenger_type.clone(),
        paired: messenger_has_credentials(m),
        // Config-level state: the messenger loop runs in the gateway
        // process; per-connection liveness isn't tracked here yet.
        online: m.enabled && messenger_has_credentials(m),
        last_message: None,
    }
}

fn channel_status(config: &Config) -> ServerFrame {
    let channels = config.messengers.iter().map(messenger_to_dto).collect();
    ServerFrame {
        frame_type: ServerFrameType::ChannelStatusResult,
        payload: ServerPayload::ChannelStatusResult { channels },
    }
}

fn channel_pair(
    config: &mut Config,
    channel: String,
    action: ChannelPairActionKind,
) -> ServerFrame {
    let result = (|| -> Result<ChannelStatusDto, String> {
        let target = config
            .messengers
            .iter_mut()
            .find(|m| messenger_display_name(m) == channel || m.messenger_type == channel)
            .ok_or_else(|| format!("Unknown channel: '{}'", channel))?;
        target.enabled = matches!(action, ChannelPairActionKind::Pair);
        let dto = messenger_to_dto(target);
        config.save(None).map_err(|e| e.to_string())?;
        Ok(dto)
    })();

    let (ok, channel, message) = match result {
        Ok(dto) => (true, Some(dto), None),
        Err(e) => (false, None, Some(e)),
    };
    ServerFrame {
        frame_type: ServerFrameType::ChannelPairResult,
        payload: ServerPayload::ChannelPairResult {
            ok,
            channel,
            message,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_schedule_forms() {
        assert!(matches!(
            parse_schedule("at 2026-07-08T12:00:00Z"),
            Ok(Schedule::At { .. })
        ));
        assert!(matches!(
            parse_schedule("every 5m"),
            Ok(Schedule::Every {
                every_ms: 300_000,
                ..
            })
        ));
        assert!(matches!(
            parse_schedule("every 1500"),
            Ok(Schedule::Every { every_ms: 1500, .. })
        ));
        assert!(matches!(
            parse_schedule("*/5 * * * *"),
            Ok(Schedule::Cron { .. })
        ));
        assert!(parse_schedule("tomorrow").is_err());
        assert!(parse_schedule("every 0s").is_err());
    }

    #[test]
    fn schedule_render_round_trip() {
        for expr in ["at 2026-07-08T12:00:00Z", "every 300000ms", "*/5 * * * *"] {
            let schedule = parse_schedule(expr).unwrap();
            assert_eq!(render_schedule(&schedule), expr);
        }
    }

    #[test]
    fn memory_entry_parsing() {
        let text = "# Memory\n\n## Preferences\n- likes rust\n* dislikes strings\n\n## Facts\n- gateway on 8080\nplain text line\n";
        let entries = parse_memory_entries(text);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].content, "likes rust");
        assert_eq!(entries[0].category.as_deref(), Some("Preferences"));
        assert_eq!(entries[1].content, "dislikes strings");
        assert_eq!(entries[2].category.as_deref(), Some("Facts"));
        // Line ids are 1-based positions in the document.
        assert_eq!(entries[0].line_idx, 3);
    }

    #[test]
    fn tool_categories_cover_registry() {
        for def in rustyclaw_core::tools::all_tools() {
            assert_ne!(
                tool_category(def.name),
                "other",
                "tool '{}' needs a panel category",
                def.name
            );
        }
    }
}
