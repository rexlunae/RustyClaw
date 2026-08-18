//! TUI gateway event types.
//!
//! These are the events the gateway-reader task sends into the iocraft
//! render loop. They are TUI-local because they describe UI state and
//! dialog prompts, not the wire protocol.

use rustyclaw_view::PromptAttachment;

/// Who a device-flow sign-in belongs to.
///
/// `None` used to stand for two different owners at once — a gateway too
/// old to attribute its frames *and* a flow this client started itself
/// (provider sign-in from `/model`). A local flow finishing then read as
/// "the flow on screen" and tore down another conversation's dialog, while
/// its own queued entry was never retired and later resurfaced with an
/// expired code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceFlowOwner {
    /// Raised by a turn, attributed to its thread by the core client.
    Turn(u64),
    /// From a gateway too old to attribute its frames — such a gateway can
    /// only run one turn, so it can only have one flow.
    Unattributed,
    /// Started by this client itself, with no turn involved.
    Local,
}

/// Deliver an event to the render component, logging the loss rather than
/// discarding it.
///
/// Every one of these is something the user is meant to see — an error, a
/// warning, a dialog opening, a list arriving. The send fails when the render
/// component is gone, which at shutdown is unremarkable and at any other time
/// means the user asked for something and got nothing back, with no
/// explanation available to them or to us. Each call site was
/// `let _ = gw_tx.send(..)`.
pub(crate) fn emit(tx: &std::sync::mpsc::Sender<GwEvent>, event: GwEvent) {
    // Named, not debug-formatted: these carry prompts, model output and
    // credential messages, none of which belong in a log line.
    let name = event.name();
    if tx.send(event).is_err() {
        rustyclaw_view::tracing::debug!(
            event = %name,
            "UI event dropped; the render component is gone (expected during shutdown)"
        );
    }
}

/// Carry a user's action back to the tokio loop, logging the loss rather than
/// discarding it.
///
/// The mirror of [`emit`], for the other direction. Every one of these is
/// something the user *did* — a prompt, a keypress answering a dialog, a Stop.
/// The send fails when the loop has ended, which at shutdown is unremarkable
/// and at any other time means the action went nowhere: no request, no error,
/// no sign on screen that anything happened. That is the shape of failure
/// hardest to diagnose from a bug report, because the person reporting it has
/// nothing to describe beyond "it did not work".
///
/// Named, not debug-formatted, for the reason [`emit`] gives: `UserInput::Chat`
/// carries the prompt the user typed, and it does not belong in a log line.
pub(crate) fn submit(
    tx: &std::sync::mpsc::Sender<super::app::UserInput>,
    input: super::app::UserInput,
) {
    let name: &'static str = (&input).into();
    if tx.send(input).is_err() {
        rustyclaw_view::tracing::debug!(
            input = %name,
            "user action dropped; the event loop is gone (expected during shutdown)"
        );
    }
}

/// Report a failed action to the log and to the user, without ending the loop.
///
/// The event loop lives in `run() -> Result<()>`, so a bare `?` there would
/// propagate out and take the whole session down because one command did not
/// go out. `?` is still the right way to *carry* the failure — this is just
/// the boundary it is allowed to stop at, which for a loop is one iteration
/// rather than the process.
///
/// The chain goes to the log via `{:?}`, which prints every `.context(..)`
/// layer that produced it, so the diagnostic names the user's action and not
/// only the send that happened to fail last.
pub(crate) fn report(tx: &std::sync::mpsc::Sender<GwEvent>, e: rustyclaw_view::anyhow::Error) {
    rustyclaw_view::tracing::error!(error = ?e, "Action failed");
    emit(
        tx,
        GwEvent::Error {
            summary: format!("Action failed: {e:#}"),
            details: Some(format!("{e:?}")),
        },
    );
}

/// Persist config, telling the user if it did not stick.
///
/// A failed save is invisible while the session lasts: the setting is already
/// applied in memory, so the toggle moves, the panel redraws, and everything
/// looks right until a restart brings the old value back. The user is the only
/// one who can act on it — check the disk, fix permissions — so unlike most of
/// these this warrants saying out loud, not just logging.
pub(crate) fn persist_config(
    config: &rustyclaw_core::config::Config,
    tx: &std::sync::mpsc::Sender<GwEvent>,
) {
    if let Err(e) = config.save(None) {
        rustyclaw_view::tracing::error!(error = %e, "Failed to save config");
        emit(
            tx,
            GwEvent::Warning {
                summary: "Setting changed for this session only — saving it failed".to_string(),
                details: Some(e.to_string()),
            },
        );
    }
}

/// Events pushed from the gateway reader into the iocraft render component.
#[derive(Debug, Clone, strum::IntoStaticStr)]
pub(crate) enum GwEvent {
    Disconnected(String),
    Connected,
    AuthChallenge,
    Authenticated,
    ModelReady(String),
    /// Gateway reloaded config — update model label in status bar.
    ModelReloaded {
        provider: String,
        model: String,
    },
    Info(String),
    Success(String),
    /// A non-fatal warning.
    Warning {
        summary: String,
        details: Option<String>,
    },
    /// An error.
    Error {
        summary: String,
        details: Option<String>,
    },
    /// A turn opened. Carries the thread the gateway is running it in, so a
    /// turn started elsewhere — or a refusal for a thread this client is not
    /// waiting on — can be told apart from the one on screen.
    StreamStart(Option<u64>),
    Chunk(Option<u64>, String),
    /// A turn closed, naming its thread. See [`GwEvent::StreamStart`].
    ResponseDone(Option<u64>),
    // Every frame of a turn is tagged with the thread it is running in.
    // With a turn per thread, reasoning text and tool panels from a
    // conversation the user is not reading would otherwise be drawn into —
    // and appended to — the one they are.
    ThinkingStart(Option<u64>),
    /// A chunk of the model's reasoning text.
    ThinkingDelta(Option<u64>, String),
    ThinkingEnd(Option<u64>),
    ToolCall {
        thread_id: Option<u64>,
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        thread_id: Option<u64>,
        id: String,
        name: String,
        result: String,
        is_error: bool,
    },
    /// Live status for a still-running tool call (elapsed + process stats).
    ToolStatus {
        thread_id: Option<u64>,
        id: String,
        status: rustyclaw_core::ui::ToolLiveStatus,
    },
    /// A chunk of live stdout/stderr from a still-running tool.
    ToolOutput {
        thread_id: Option<u64>,
        id: String,
        chunk: String,
    },
    /// Gateway requests user approval for a tool call (Ask mode). Tagged
    /// with the asking turn's thread: call ids collide across turns, so
    /// the tool's result retires its own turn's entry by (thread, id).
    ToolApprovalRequest {
        thread_id: Option<u64>,
        id: String,
        name: String,
        arguments: String,
    },
    /// Gateway requests structured user input (ask_user tool). Thread-
    /// tagged like `ToolApprovalRequest`, and for the same reason.
    UserPromptRequest {
        thread_id: Option<u64>,
        prompt: rustyclaw_core::user_prompt_types::UserPrompt,
    },
    /// Gateway requests an API key or credential from the user. Tagged with
    /// the asking turn's thread: a credential wait ending is what ends its
    /// turn, so the turn's close-out retires requests it left behind.
    CredentialRequest {
        thread_id: Option<u64>,
        id: String,
        provider: String,
        secret_name: String,
        message: String,
    },
    /// Vault is locked — user needs to provide password.
    VaultLocked,
    /// Vault was successfully unlocked.
    VaultUnlocked,
    /// Show secrets info dialog.
    ShowSecrets {
        secrets: Vec<rustyclaw_view::SecretInfoData>,
        agent_access: bool,
        has_totp: bool,
    },
    /// Show skills info dialog.
    ShowSkills {
        skills: Vec<rustyclaw_view::SkillInfoData>,
    },
    /// Show tool permissions info dialog.
    ShowToolPerms {
        tools: Vec<rustyclaw_view::ToolPermInfoData>,
    },
    /// A secrets mutation succeeded — re-fetch the list from the gateway.
    RefreshSecrets,
    /// Outcome of a secret reveal: either the plaintext fields, or a request
    /// for the TOTP code that gates them.
    SecretRevealed {
        ok: bool,
        fields: Vec<(String, String)>,
        message: Option<String>,
        totp_required: bool,
    },
    /// Thread list update from gateway (unified tasks + threads).
    ThreadsUpdate {
        threads: Vec<crate::action::ThreadInfo>,
        foreground_id: Option<u64>,
    },

    /// Project list update (the sidebar's top level).
    ProjectsUpdate {
        projects: Vec<rustyclaw_core::ui::ProjectInfo>,
        active_id: u64,
    },

    /// Agent list update (agents in this installation + the active one).
    AgentsUpdate {
        agents: Vec<rustyclaw_view::AgentItemData>,
        active_id: String,
    },

    /// The connection's active agent changed.
    AgentSwitched {
        agent_id: String,
        name: String,
    },

    /// Open the agent selector dialog (/agents).
    ShowAgentSelector,

    /// Live message/history update for a thread.
    ThreadMessages {
        thread_id: u64,
        messages: Vec<rustyclaw_core::gateway::protocol::types::ChatMessage>,
    },

    /// Thread switch confirmed — clear messages and show context.
    ThreadSwitched {
        thread_id: u64,
        context_summary: Option<String>,
    },
    /// Authoritative thread history from the gateway, in chronological order.
    ThreadHistory {
        thread_id: u64,
        ok: bool,
        messages: Vec<rustyclaw_core::gateway::protocol::types::ChatMessage>,
        error: Option<String>,
    },
    /// Open the provider selector dialog.
    ShowProviderSelector {
        providers: Vec<String>,
        provider_ids: Vec<String>,
        auth_hints: Vec<String>,
    },
    /// Open the API key input dialog.
    PromptApiKey {
        provider: String,
        provider_display: String,
        help_url: String,
        help_text: String,
    },
    /// Show the device flow verification dialog. Tagged with the flow's
    /// owner, so completing or ending one sign-in cannot tear down
    /// another's.
    DeviceFlowCode {
        owner: DeviceFlowOwner,
        provider: String,
        url: String,
        code: String,
    },
    /// Device flow completed — dismiss its dialog and retire its queue
    /// entry.
    DeviceFlowDone(DeviceFlowOwner),
    /// Device flow succeeded — store token and proceed to model selection.
    DeviceFlowToken {
        provider: String,
        token: String,
    },
    /// Open the model selector dialog.
    ShowModelSelector {
        provider: String,
        provider_display: String,
        models: Vec<String>,
    },
    /// Prompt attachment list changed.
    PromptAttachmentsChanged {
        attachments: Vec<PromptAttachment>,
    },
    /// Live model IDs loaded for slash-command autocomplete.
    ModelCompletionsLoaded {
        provider: String,
        models: Vec<String>,
    },
    /// Hugging Face Hub repo ids loaded for model-name autocomplete
    /// (`/engines pull|files|search …`). `query` echoes the search text
    /// the results answer, so stale responses can be told apart.
    HubModelCompletionsLoaded {
        query: String,
        models: Vec<String>,
    },
    /// Model fetch is in progress (show loading spinner).
    FetchModelsLoading {
        provider: String,
        provider_display: String,
    },
    /// SSH pairing connection succeeded.
    PairingSuccess {
        gateway_name: String,
    },
    /// SSH pairing connection failed.
    PairingError(String),
    /// Open the local engines panel (data arrives via EngineListResult).
    ShowEngines,
    /// Engine list result received.
    EngineListResult {
        engines: Vec<rustyclaw_view::LocalEngineData>,
    },
    /// Full per-engine configuration, keyed by engine id (arrives right
    /// after `EngineListResult`; patches the engine entries' configs).
    EngineConfigList {
        configs: std::collections::HashMap<String, rustyclaw_core::engines::EngineConfig>,
    },
    /// Engine model list result received.
    EngineModelListResult {
        engine: String,
        models: Vec<rustyclaw_view::LocalModelData>,
    },
    /// Pull progress update.
    EnginePullProgress {
        engine: String,
        model: String,
        percent: f32,
        downloaded_bytes: u64,
        total_bytes: u64,
        status: String,
    },
    /// Engine action completed.
    EngineActionResult {
        engine: String,
        #[allow(dead_code)]
        model: Option<String>,
        ok: bool,
        message: String,
    },
    /// Streamed output line from an in-progress engine action (install).
    EngineActionProgress {
        engine: String,
        line: String,
    },
    /// Clear the message display (/clear).
    ClearMessages,
    /// Show gateway connection status (/gateway).
    ShowGatewayStatus,
    /// Open the downloads panel (data arrives via DownloadsUpdate).
    ShowDownloads,
    /// Transfer list received. Arrives whenever one changes, not only when
    /// asked for, so the panel moves without polling.
    DownloadsUpdate {
        downloads: Vec<rustyclaw_view::DownloadData>,
    },
    /// Open the cron panel (data arrives via CronListResult).
    ShowCron,
    /// Cron job list received.
    CronListResult {
        jobs: Vec<rustyclaw_view::CronJobData>,
    },
    /// Open the memory panel with an optional filter query.
    ShowMemory {
        query: Option<String>,
    },
    /// Memory entry list received.
    MemoryListResult {
        entries: Vec<rustyclaw_view::MemoryEntryData>,
    },
    /// History search results received (shown in the memory panel).
    HistorySearchResult {
        entries: Vec<rustyclaw_view::HistoryEntryData>,
    },
    /// Open the MCP servers panel (data arrives via McpListResult).
    ShowMcp,
    /// MCP server list received.
    McpListResult {
        servers: Vec<rustyclaw_view::McpServerData>,
    },
    /// Open the messenger channels panel.
    ShowChannels,
    /// Channel status list received.
    ChannelStatusResult {
        channels: Vec<rustyclaw_view::ChannelStatusData>,
    },
    /// Open the messenger setup panel.
    ShowMessengers,
    /// The messenger setup view arrived — accounts, routes, routable threads.
    MessengerConfigResult {
        accounts: Vec<rustyclaw_view::MessengerAccountData>,
        routes: Vec<rustyclaw_view::MessengerRouteData>,
        threads: Vec<rustyclaw_view::RoutableThreadData>,
        available_kinds: Vec<String>,
        vault_locked: bool,
    },
    /// A messenger account or route mutation finished.
    MessengerActionResult {
        ok: bool,
        message: Option<String>,
    },
    /// A panel mutation finished — show the outcome and re-fetch the list.
    PanelActionResult {
        panel: PanelKind,
        ok: bool,
        message: Option<String>,
    },
    /// Open the analytics panel (data arrives via UsageStatsResult).
    ShowAnalytics,
    /// Usage stats received.
    UsageStatsResult {
        totals: rustyclaw_view::UsageTotalsData,
        per_model: Vec<rustyclaw_view::ModelUsageData>,
        per_session: Vec<rustyclaw_view::SessionUsageData>,
    },
    /// Open the logs panel (data arrives via LogsResult).
    ShowLogs {
        source: String,
    },
    /// Log lines received.
    LogsResult {
        ok: bool,
        source: String,
        lines: Vec<String>,
        message: Option<String>,
    },
    /// Host hardware capabilities received.
    HostInfo(rustyclaw_view::HostInfoData),
    /// Current system load status received.
    LoadStatus(rustyclaw_view::LoadStatusData),
    /// Service list received from gateway.
    ServiceList(rustyclaw_view::ServiceListData),
    /// A single service was updated (start/stop/restart result).
    ServiceActionResult {
        service: Option<rustyclaw_view::ServiceInfoData>,
    },
}

impl GwEvent {
    /// The variant's name, carrying none of its payload.
    ///
    /// Same reasoning as `GatewayCommand::name`: these events hold model
    /// output, prompts and credential messages, so debug-formatting one into
    /// a log writes user content to disk. Derived from the debug rendering
    /// rather than matched arm by arm so it cannot drift as variants are
    /// added.
    pub(crate) fn name(&self) -> &'static str {
        // A generated match over the variants, so the payload is never
        // rendered. `{:?}` here would materialise model output, prompts and
        // credential messages into a temporary just to read the identifier
        // off the front of it.
        self.into()
    }
}

/// Which gateway panel a [`GwEvent::PanelActionResult`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanelKind {
    Cron,
    Memory,
    Mcp,
    Channels,
}

impl PanelKind {
    /// Panel display name for status messages.
    pub fn label(self) -> &'static str {
        match self {
            PanelKind::Cron => "Cron",
            PanelKind::Memory => "Memory",
            PanelKind::Mcp => "MCP",
            PanelKind::Channels => "Channels",
        }
    }
}

impl GwEvent {
    /// Warning event with no extended details.
    pub fn warning(summary: impl Into<String>) -> Self {
        GwEvent::Warning {
            summary: summary.into(),
            details: None,
        }
    }

    /// Error event with no extended details.
    pub fn error(summary: impl Into<String>) -> Self {
        GwEvent::Error {
            summary: summary.into(),
            details: None,
        }
    }

    /// Error event from an `anyhow_tracing::Error`.
    pub fn error_from_err(err: &anyhow_tracing::Error) -> Self {
        GwEvent::Error {
            summary: format!("{:#}", err),
            details: Some(rustyclaw_core::error_details::render_extended(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::app::UserInput;

    /// A user action whose receiver has gone must not take the UI down with
    /// it. The loop ending is the normal shutdown order, and a panic here
    /// would turn every quit into a crash.
    #[test]
    fn submitting_to_a_closed_loop_is_survivable() {
        let (tx, rx) = std::sync::mpsc::channel::<UserInput>();
        drop(rx);

        submit(
            &tx,
            UserInput::Chat {
                text: "hello".into(),
                thread_id: Some(1),
            },
        );
    }

    /// The log line names the variant and nothing else. `UserInput::Chat`
    /// carries the prompt the user typed; `{:?}` on it would put that in the
    /// log, which is the thing `emit` avoids for the same reason.
    #[test]
    fn the_name_identifies_without_quoting_the_user() {
        let input = UserInput::Chat {
            text: "my private prompt".into(),
            thread_id: None,
        };
        let name: &'static str = (&input).into();

        assert_eq!(name, "Chat");
        assert!(
            !name.contains("private"),
            "the variant name must not carry the payload"
        );
    }

    /// A live receiver still gets the action — the logging wrapper must not
    /// have become a silent drop of its own.
    #[test]
    fn a_live_loop_receives_the_action() {
        let (tx, rx) = std::sync::mpsc::channel::<UserInput>();

        submit(&tx, UserInput::Quit);

        assert!(
            matches!(rx.try_recv(), Ok(UserInput::Quit)),
            "the action should have arrived"
        );
    }
}
