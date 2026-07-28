//! Application state management.
//!
//! Shared UI types (`ChatMessage`, `ToolCallInfo`, `ThreadInfo`,
//! `ConnectionStatus`) live in [`rustyclaw_core::ui`]. This module
//! adds desktop-specific wrappers: the Dioxus-friendly `AppState` struct
//! and the `Theme` enum.

use std::collections::{HashMap, HashSet, VecDeque};

use rustyclaw_core::gateway::protocol;
use rustyclaw_core::ui::{ChatMessage, ConnectionStatus, ThreadInfo};
use rustyclaw_core::user_prompt_types::UserPrompt;
use rustyclaw_view::{PromptAttachment, SecretsDialogData};
use rustyclaw_view::{chrono, tracing, uuid};

/// A workspace change held back while the user decides what to do with
/// unsaved editor changes.
///
/// Every one of these repoints the thread's working directory, which
/// invalidates the editor's caches — so the decision has to be made before the
/// change is applied, not after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingWorkspaceChange {
    /// Change the working directory to this path.
    Directory(String),
    /// Make this project active.
    Project(u64),
    /// Bring this thread to the foreground.
    Thread(u64),
}

/// UI theme preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub fn as_attr(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }
}

/// Main application state.
#[derive(Clone, Debug)]
pub struct AppState {
    /// Current connection status
    pub connection: ConnectionStatus,

    /// Gateway URL
    pub gateway_url: String,

    /// Chat messages for the current thread
    pub messages: VecDeque<ChatMessage>,

    /// Per-thread message history (thread_id → messages)
    thread_messages: HashMap<u64, VecDeque<ChatMessage>>,

    /// Whether we're waiting for a response
    pub is_processing: bool,

    /// Whether the assistant is currently streaming
    pub is_streaming: bool,

    /// Current thinking state (for extended thinking models)
    pub is_thinking: bool,

    /// When the current thinking block began (for "Thought for Xs").
    pub thinking_started: Option<std::time::Instant>,

    /// Start times of in-flight tool calls, by tool-call id, so results
    /// can be stamped with a wall-clock duration.
    pub tool_started: HashMap<String, std::time::Instant>,

    /// Active threads/sessions
    pub threads: Vec<ThreadInfo>,

    /// Known projects (the sidebar's top level)
    pub projects: Vec<rustyclaw_core::ui::ProjectInfo>,

    /// The active project's ID
    pub active_project_id: u64,

    /// Current foreground thread ID
    pub foreground_thread_id: Option<u64>,

    /// The thread the in-flight response belongs to (set at submit time,
    /// cleared on completion). Stream events carry no thread id on the wire,
    /// so this is how the client knows whether live stream events target the
    /// thread currently on screen or one the user has switched away from.
    pub streaming_thread_id: Option<u64>,

    /// Agent name from hatching
    pub agent_name: Option<String>,

    /// Whether vault is locked
    pub vault_locked: bool,

    /// Whether we need to show hatching dialog
    pub needs_hatching: bool,

    /// Current model name
    pub model: Option<String>,

    /// Current provider name
    pub provider: Option<String>,

    /// Files and directories attached to the next prompt.
    pub prompt_attachments: Vec<PromptAttachment>,

    /// Whether the sidebar is collapsed.
    pub sidebar_collapsed: bool,

    /// Active UI theme.
    pub theme: Theme,

    /// Pending tool approval request (id, name, arguments).
    pub pending_tool_approval: Option<(String, String, String)>,

    /// Pending user prompt from the agent.
    pub pending_user_prompt: Option<UserPrompt>,

    /// Pending credential request (id, provider, secret_name, message).
    pub pending_credential_request: Option<(String, String, String, String)>,

    /// Pending device flow (url, code, message).
    pub pending_device_flow: Option<(String, String, Option<String>)>,

    /// Number of streaming chunks received in the current response.
    pub streaming_chunks: u32,

    /// Total bytes received in the current streaming response.
    pub streaming_bytes: usize,

    /// Whether the agent currently has access to vault secrets.
    pub agent_access: bool,

    /// Current secrets dialog data.
    pub secrets_data: SecretsDialogData,

    /// Current working directory path
    pub working_directory: Option<String>,

    /// Available directories for selection (favorites/recent)
    pub available_directories: Vec<rustyclaw_view::DirectoryOption>,

    /// Whether the directory selector is expanded
    pub directory_selector_expanded: bool,

    /// Error message from directory operations if any
    pub directory_selector_error: Option<String>,

    /// Whether the left sidebar (thread list) is visible.
    pub left_sidebar_visible: bool,

    /// Whether the plugin dock (the workspace's right-hand column) is shown.
    pub plugin_dock_visible: bool,

    /// File tree for the workspace directory. Kept up to date as the working
    /// directory changes; the editor plugin renders it.
    pub file_browser: rustyclaw_view::FileBrowserData,

    /// Directory listings from the thread's working directory, keyed by the
    /// directory's path relative to it. Populated by `WorkspaceDirListing`.
    pub workspace_listings: std::collections::HashMap<
        std::path::PathBuf,
        Vec<rustyclaw_core::gateway::WorkspaceEntryDto>,
    >,

    /// Contents of files opened from the thread's working directory, keyed by
    /// path relative to it. Populated by `WorkspaceFileContent`.
    pub workspace_files: std::collections::HashMap<std::path::PathBuf, String>,

    /// Directories the editor's tree has expanded, relative to the thread's
    /// working directory. The root (`""`) starts expanded.
    pub editor_expanded: std::collections::HashSet<std::path::PathBuf>,

    /// Files the editor has open, in tab order.
    pub editor_open: Vec<std::path::PathBuf>,

    /// The tab the editor is showing.
    pub editor_active: Option<std::path::PathBuf>,

    /// Edited contents, keyed by path. Present only once the user has typed:
    /// a file with no entry here is unmodified, so `is_dirty` needs no
    /// separate flag to fall out of sync with.
    pub editor_edits: std::collections::HashMap<std::path::PathBuf, String>,

    /// Contents of saves the editor has sent but not yet heard back about,
    /// keyed by path. `WorkspaceWriteResult` reports only path/ok/error, so
    /// the written text has to be remembered here to reconcile the buffer
    /// once the save lands.
    pub editor_saving: std::collections::HashMap<std::path::PathBuf, String>,

    /// A workspace change waiting on the unsaved-changes prompt.
    pub pending_workspace_change: Option<PendingWorkspaceChange>,

    /// Bumped every time the workspace view is reset.
    ///
    /// The gateway never pushes a directory listing — it only answers a
    /// request — so after a reset something has to ask again. Watching a
    /// counter rather than "is the cache empty" means a directory that
    /// genuinely lists as empty, or one whose listing failed, does not
    /// re-request forever.
    pub workspace_generation: u64,

    /// Plugin snapshots for the plugin panel.
    pub plugins: Vec<crate::components::PluginSnapshot>,

    /// Active plugin name in the plugin panel.
    pub active_plugin: Option<String>,

    /// Gateway host hardware capabilities.
    pub host_info: Option<rustyclaw_view::HostInfoData>,

    /// Current system load status.
    pub load_status: Option<rustyclaw_view::LoadStatusData>,

    /// Whether the system info panel is visible.
    pub show_system_info: bool,

    /// Whether the services dialog is visible.
    pub show_services_dialog: bool,

    /// Service list data for the services dialog.
    pub services_data: Option<rustyclaw_view::ServiceListData>,

    /// Whether the local engines/models dialog is visible.
    pub show_engines_dialog: bool,

    /// Local engine + model data for the engines dialog.
    pub engines_data: Option<rustyclaw_view::EnginesPanelData>,

    /// Set when an engine action completed and the engine/model lists
    /// should be re-fetched from the gateway.
    pub engines_stale: bool,

    /// Whether the scheduled-jobs dialog is visible.
    pub show_cron_dialog: bool,
    /// Cron job data for the scheduled-jobs dialog.
    pub cron_data: Option<rustyclaw_view::CronPanelData>,
    /// Set when a cron mutation completed and the list should be re-fetched.
    pub cron_stale: bool,

    /// Whether the memory browser dialog is visible.
    pub show_memory_dialog: bool,
    /// Memory entry data for the memory browser dialog.
    pub memory_data: Option<rustyclaw_view::MemoryPanelData>,
    /// Set when a memory mutation completed and the list should be re-fetched.
    pub memory_stale: bool,

    /// Whether the MCP servers dialog is visible.
    pub show_mcp_dialog: bool,
    /// MCP server data for the MCP dialog.
    pub mcp_data: Option<rustyclaw_view::McpPanelData>,
    /// Set when an MCP mutation completed and the list should be re-fetched.
    pub mcp_stale: bool,

    /// Whether the messenger channels dialog is visible.
    pub show_channels_dialog: bool,
    /// Channel status data for the channels dialog.
    pub channels_data: Option<rustyclaw_view::ChannelsPanelData>,
    /// Set when a channel mutation completed and the list should be re-fetched.
    pub channels_stale: bool,

    /// Whether the tool permissions dialog is visible.
    pub show_tools_dialog: bool,
    /// Tool configuration data for the tool permissions dialog.
    pub tools_data: Option<rustyclaw_view::ToolConfigPanelData>,
    /// Set when a tool toggle completed and the list should be re-fetched.
    pub tools_stale: bool,

    /// User-defined custom providers from the local config (shown and
    /// edited in Settings).
    pub custom_providers: Vec<rustyclaw_core::providers::CustomProviderConfig>,

    /// Whether the skills manager dialog is visible.
    pub show_skills_dialog: bool,
    /// Skills for the skills manager dialog.
    pub skills_data: Vec<rustyclaw_view::SkillInfoData>,

    /// Whether the usage analytics dialog is visible.
    pub show_analytics_dialog: bool,
    /// Usage analytics data.
    pub analytics_data: Option<rustyclaw_view::AnalyticsPanelData>,

    /// Whether the logs dialog is visible.
    pub show_logs_dialog: bool,
    /// Log lines for the logs dialog.
    pub logs_data: Option<rustyclaw_view::LogsPanelData>,

    /// Live model lists fetched from provider APIs (via the gateway),
    /// keyed by provider id.  The model picker prefers these over the
    /// static catalogue fallback.
    pub provider_models: HashMap<String, Vec<String>>,
    /// Providers whose live model list has already been requested this
    /// session (guards against duplicate in-flight requests).
    pub provider_models_requested: HashSet<String>,
}

impl Default for AppState {
    fn default() -> Self {
        let working_directory = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string());
        let configured_model = rustyclaw_core::config::Config::load(None)
            .ok()
            .and_then(|cfg| cfg.model);
        let provider = configured_model.as_ref().map(|m| m.provider.clone());
        let model = configured_model.and_then(|m| m.model);

        // Check whether SOUL.md needs first-run setup.
        let needs_hatching = rustyclaw_core::config::Config::load(None)
            .ok()
            .map(|cfg| {
                let mut sm = rustyclaw_core::soul::SoulManager::new(cfg.soul_path());
                let _ = sm.load();
                sm.needs_hatching()
            })
            .unwrap_or(false);

        Self {
            connection: ConnectionStatus::Disconnected,
            gateway_url: crate::configured_gateway_url()
                .or_else(crate::load_saved_gateway_url)
                .unwrap_or_else(|| crate::DEFAULT_GATEWAY_URL.to_string()),
            messages: VecDeque::new(),
            thread_messages: HashMap::new(),
            is_processing: false,
            is_streaming: false,
            is_thinking: false,
            thinking_started: None,
            tool_started: HashMap::new(),
            projects: Vec::new(),
            active_project_id: 0,
            threads: Vec::new(),
            foreground_thread_id: None,
            streaming_thread_id: None,
            agent_name: None,
            vault_locked: false,
            needs_hatching,
            model,
            provider,
            prompt_attachments: Vec::new(),
            sidebar_collapsed: false,
            theme: Theme::default(),
            pending_tool_approval: None,
            pending_user_prompt: None,
            pending_credential_request: None,
            pending_device_flow: None,
            streaming_chunks: 0,
            streaming_bytes: 0,
            agent_access: false,
            secrets_data: SecretsDialogData::from_vault(Vec::new(), false, false),
            working_directory: working_directory.clone(),
            available_directories: Vec::new(),
            directory_selector_expanded: false,
            directory_selector_error: None,
            left_sidebar_visible: true,
            plugin_dock_visible: true,
            workspace_listings: Default::default(),
            workspace_files: Default::default(),
            editor_expanded: Default::default(),
            editor_open: Vec::new(),
            editor_active: None,
            editor_edits: Default::default(),
            editor_saving: Default::default(),
            pending_workspace_change: None,
            workspace_generation: 0,
            file_browser: working_directory
                .as_deref()
                .map(rustyclaw_view::FileBrowserData::load)
                .unwrap_or_default(),
            plugins: Vec::new(),
            active_plugin: None,
            host_info: None,
            load_status: None,
            show_system_info: false,
            show_services_dialog: false,
            services_data: None,
            show_engines_dialog: false,
            engines_data: None,
            engines_stale: false,
            show_cron_dialog: false,
            cron_data: None,
            cron_stale: false,
            show_memory_dialog: false,
            memory_data: None,
            memory_stale: false,
            show_mcp_dialog: false,
            mcp_data: None,
            mcp_stale: false,
            show_channels_dialog: false,
            channels_data: None,
            channels_stale: false,
            show_tools_dialog: false,
            tools_data: None,
            tools_stale: false,
            custom_providers: rustyclaw_core::config::Config::load(None)
                .map(|cfg| cfg.custom_providers)
                .unwrap_or_default(),
            show_skills_dialog: false,
            skills_data: Vec::new(),
            show_analytics_dialog: false,
            analytics_data: None,
            show_logs_dialog: false,
            logs_data: None,
            provider_models: HashMap::new(),
            provider_models_requested: HashSet::new(),
        }
    }
}

impl AppState {
    /// Add a user message to the conversation.
    pub fn add_user_message(&mut self, content: String) {
        let msg = ChatMessage::user(content);
        self.messages.push_back(msg);
    }

    /// Append an inline notice banner (Info/Success/Warning/Error) to the
    /// transcript. Notices render in the chat at the point they occurred,
    /// replacing the old transient status snackbar.
    pub fn push_notice(
        &mut self,
        role: rustyclaw_core::types::MessageRole,
        content: impl Into<String>,
    ) {
        self.messages.push_back(ChatMessage::notice(role, content));
    }

    /// Mark a request as submitted: the response that follows belongs to the
    /// current foreground thread. Stream events are applied to the live view
    /// only while that thread stays in the foreground.
    pub fn mark_request_started(&mut self) {
        self.is_processing = true;
        self.streaming_thread_id = self.foreground_thread_id;
    }

    /// Whether live stream events (StreamStart/Chunk/Thinking/ToolCall…)
    /// target the thread currently on screen. `None` means the response
    /// thread is unknown (e.g. submitted before any thread existed) and
    /// events apply to whatever is in the foreground.
    pub fn stream_targets_foreground(&self) -> bool {
        self.streaming_thread_id.is_none() || self.streaming_thread_id == self.foreground_thread_id
    }

    /// Start a new assistant message (streaming).
    pub fn start_assistant_message(&mut self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let msg = ChatMessage::start_assistant(id.clone());
        self.messages.push_back(msg);
        self.is_streaming = true;
        self.streaming_chunks = 0;
        self.streaming_bytes = 0;
        id
    }

    /// Append content to the current streaming assistant message.
    ///
    /// The newest message may be a folded thinking block (reasoning
    /// closes the moment the first answer chunk arrives), so search from
    /// the rear for the streaming assistant bubble — and start a fresh
    /// one when the turn doesn't have one yet, so answer text arriving
    /// after a thinking block is never dropped.
    pub fn append_to_current_message(&mut self, delta: &str) {
        if let Some(msg) = self
            .messages
            .iter_mut()
            .rev()
            .find(|m| m.is_streaming && m.role == rustyclaw_core::types::MessageRole::Assistant)
        {
            msg.append_content(delta);
            return;
        }
        self.start_assistant_message();
        if let Some(msg) = self.messages.back_mut() {
            msg.append_content(delta);
        }
    }

    /// Finish the current streaming message(s). Marks every message
    /// still flagged as streaming finished — the answer bubble may not
    /// be last (e.g. a thinking block folded after it).
    pub fn finish_current_message(&mut self) {
        for msg in self.messages.iter_mut() {
            msg.finish();
        }
        self.is_streaming = false;
        self.is_processing = false;
        self.streaming_chunks = 0;
        self.streaming_bytes = 0;
        self.streaming_thread_id = None;
    }

    /// Handle the end of a response. Finalizes the live view only when the
    /// response targeted the foreground thread; a response that completed in
    /// a backgrounded thread just releases the in-flight marker (its
    /// transcript arrives via the gateway's history snapshot).
    pub fn response_done(&mut self) {
        if self.stream_targets_foreground() {
            self.finish_current_message();
        } else {
            self.streaming_thread_id = None;
        }
    }

    /// Add a tool call to the current turn and start its clock. Like
    /// answer text, tool calls belong to the streaming assistant bubble,
    /// not to a folded thinking block that may sit after it.
    pub fn add_tool_call(&mut self, id: String, name: String, arguments: String) {
        self.tool_started
            .insert(id.clone(), std::time::Instant::now());
        if let Some(msg) = self
            .messages
            .iter_mut()
            .rev()
            .find(|m| m.is_streaming && m.role == rustyclaw_core::types::MessageRole::Assistant)
        {
            msg.add_tool_call(id, name, arguments);
            return;
        }
        self.start_assistant_message();
        if let Some(msg) = self.messages.back_mut() {
            msg.add_tool_call(id, name, arguments);
        }
    }

    /// Update the live status of a still-running tool call.
    pub fn set_tool_live_status(&mut self, id: &str, status: rustyclaw_core::ui::ToolLiveStatus) {
        for msg in self.messages.iter_mut().rev() {
            if msg.set_tool_live_status(id, status.clone()) {
                return;
            }
        }
    }

    /// Append live output to a still-running tool call, wherever its
    /// message sits.
    pub fn append_tool_output(&mut self, id: &str, chunk: &str) {
        for msg in self.messages.iter_mut().rev() {
            if msg.append_tool_output(id, chunk) {
                return;
            }
        }
    }

    /// Set the result for a tool call, stamping the wall-clock duration
    /// measured since the matching `add_tool_call`.
    pub fn set_tool_result(&mut self, id: &str, result: String, is_error: bool) {
        let duration_ms = self
            .tool_started
            .remove(id)
            .map(|t| t.elapsed().as_millis() as u64);
        for msg in self.messages.iter_mut().rev() {
            msg.set_tool_result(id, result.clone(), is_error, duration_ms);
            // If this message had the matching tool call, the set was done.
            // We only need to check if it updated, but for simplicity just scan.
        }
    }

    /// Open a thinking block: push a streaming Thinking message that
    /// accumulates reasoning deltas. Any block left open by a dropped
    /// stream is folded first, so only one block is ever open.
    ///
    /// Reasoning precedes the answer it produces, so the block must
    /// render above the answer text: the empty assistant bubble that
    /// StreamStart opened is dropped (Chunk re-creates one after the
    /// block), and a bubble that already has content is finished so
    /// later text starts a fresh bubble below the block.
    pub fn start_thinking_message(&mut self) {
        self.end_thinking_message();
        let tail_is_empty_assistant = self.messages.back().is_some_and(|m| {
            m.role == rustyclaw_core::types::MessageRole::Assistant
                && m.is_streaming
                && m.content.is_empty()
                && m.tool_calls.is_empty()
        });
        if tail_is_empty_assistant {
            self.messages.pop_back();
        } else if let Some(m) = self.messages.back_mut() {
            m.finish();
        }
        self.is_thinking = true;
        self.thinking_started = Some(std::time::Instant::now());
        self.messages.push_back(ChatMessage::start_thinking());
    }

    /// Append reasoning text to the open thinking block (no-op when the
    /// latest message isn't a streaming Thinking block).
    pub fn append_thinking(&mut self, delta: &str) {
        if let Some(msg) = self.messages.back_mut()
            && msg.role == rustyclaw_core::types::MessageRole::Thinking
        {
            msg.content.push_str(delta);
        }
    }

    /// Close the thinking block: stamp its duration, finish streaming,
    /// and drop it entirely if the provider sent no reasoning text.
    /// The open block is usually last, but answer chunks may already have
    /// started a new assistant bubble after it — search from the rear for
    /// the newest thinking block not yet closed out.
    pub fn end_thinking_message(&mut self) {
        self.is_thinking = false;
        let duration_ms = self
            .thinking_started
            .take()
            .map(|t| t.elapsed().as_millis() as u64);
        let Some(idx) = self.messages.iter().rposition(|m| {
            m.role == rustyclaw_core::types::MessageRole::Thinking && m.is_streaming
        }) else {
            return;
        };
        if self.messages[idx].content.trim().is_empty() {
            self.messages.remove(idx);
        } else if let Some(msg) = self.messages.get_mut(idx) {
            msg.duration_ms = duration_ms;
            msg.is_streaming = false;
        }
    }

    /// Save messages for a specific thread.
    pub fn save_thread_messages(&mut self, thread_id: u64, messages: VecDeque<ChatMessage>) {
        self.thread_messages.insert(thread_id, messages);
    }

    /// Whether a request is in flight *for the thread on screen* (waiting,
    /// thinking, or streaming). While true, history snapshots from the
    /// gateway must not replace the live view: doing so would drop the
    /// in-flight streaming bubble and clear the busy indicators, making the
    /// agent look idle while it is still working. The gateway sends another
    /// snapshot when the response completes. A request running in a
    /// *backgrounded* thread never blocks the foreground view.
    pub fn foreground_request_in_flight(&self) -> bool {
        (self.is_processing || self.is_streaming || self.is_thinking)
            && self.stream_targets_foreground()
    }

    /// The gateway's sentinel thread id for "no thread is focused"; carries
    /// an empty message list to clear the view.
    const NO_THREAD: u64 = 0;

    /// Whether a history snapshot for `thread_id` should replace what is on
    /// screen.
    ///
    /// Yes when it belongs to the thread being viewed, and also when this
    /// client doesn't know its foreground thread yet: history replies can
    /// arrive before the `ThreadsUpdate` that names the foreground, and a
    /// snapshot the gateway sent unprompted is better than an empty
    /// transcript. In both cases an in-flight request wins — replacing the
    /// view mid-response would drop the streaming bubble.
    fn history_should_take_the_view(&self, thread_id: u64) -> bool {
        let targets_view = self.foreground_thread_id == Some(thread_id)
            || (self.foreground_thread_id.is_none() && thread_id != Self::NO_THREAD);
        targets_view && !self.foreground_request_in_flight()
    }

    /// Replace the cached messages for a thread with an authoritative
    /// history from the gateway. If the thread is the one on screen, also
    /// refresh the live view.
    pub fn apply_thread_history(&mut self, thread_id: u64, messages: VecDeque<ChatMessage>) {
        self.thread_messages.insert(thread_id, messages.clone());
        if self.history_should_take_the_view(thread_id) {
            tracing::debug!(
                thread_id,
                msg_count = messages.len(),
                "applying thread history to the view"
            );
            self.messages = messages;
            self.foreground_thread_id = Some(thread_id);
            self.reset_streaming_indicators();
        } else {
            // History arriving for the thread the user is *looking at* and
            // still not being shown is the shape of a bug, not routine
            // caching, so it is logged loudly enough to appear in a normal
            // run. The usual cause is a stale in-flight flag, which parks the
            // snapshot in the cache and leaves the pane blank.
            let targets_view = self.foreground_thread_id == Some(thread_id);
            let level_msg = "thread history arrived but was not displayed";
            if targets_view {
                tracing::warn!(
                    thread_id,
                    msg_count = messages.len(),
                    in_flight = self.foreground_request_in_flight(),
                    is_processing = self.is_processing,
                    is_streaming = self.is_streaming,
                    is_thinking = self.is_thinking,
                    streaming_thread = ?self.streaming_thread_id,
                    "{level_msg}"
                );
            } else {
                tracing::debug!(
                    thread_id,
                    msg_count = messages.len(),
                    foreground = ?self.foreground_thread_id,
                    "caching thread history for a thread that is not on screen"
                );
            }
        }
    }

    /// Point the view at the gateway's foreground thread.
    ///
    /// Called when a `ThreadsUpdate` names the foreground. If a history
    /// snapshot for that thread already arrived — the gateway sends one
    /// unprompted on connect, and it can land before the thread list — show
    /// it now rather than waiting for another round trip that may never come.
    pub fn set_foreground_thread(&mut self, thread_id: Option<u64>) {
        if self.foreground_thread_id == thread_id {
            return;
        }
        if let Some(outgoing) = self.foreground_thread_id
            && !self.messages.is_empty()
        {
            self.thread_messages.insert(outgoing, self.messages.clone());
        }
        self.foreground_thread_id = thread_id;
        if self.foreground_request_in_flight() {
            return;
        }
        if let Some(id) = thread_id
            && let Some(cached) = self.thread_messages.get(&id)
        {
            self.messages = cached.clone();
            self.reset_streaming_indicators();
        }
    }

    /// Replace a thread's messages with canonical history from the gateway.
    pub fn hydrate_thread_messages(
        &mut self,
        thread_id: u64,
        messages: Vec<protocol::types::ChatMessage>,
    ) {
        let hydrated = ui_history_from_gateway(messages);
        // `thread_id == 0` is the gateway's "nothing is focused" sentinel: it
        // carries an empty list to clear the view, and must not be cached or
        // adopted as a real thread.
        if thread_id == Self::NO_THREAD {
            if !self.foreground_request_in_flight() {
                self.messages.clear();
                self.reset_streaming_indicators();
            }
            return;
        }
        self.thread_messages.insert(thread_id, hydrated.clone());
        if self.history_should_take_the_view(thread_id) {
            self.messages = hydrated;
            self.foreground_thread_id = Some(thread_id);
            self.reset_streaming_indicators();
        }
    }

    /// Switch to a different thread, saving current messages and
    /// restoring the target thread's history.
    /// Files the editor has unsaved changes for.
    ///
    /// An edit that merely matches what was loaded is not unsaved, so this
    /// names real losses only — the same rule [`Self::reset_workspace_view`]
    /// reports by, kept in one place so the prompt and the warning cannot
    /// disagree about what is at stake.
    pub fn unsaved_editor_files(&self) -> Vec<std::path::PathBuf> {
        let mut files: Vec<std::path::PathBuf> = self
            .editor_edits
            .iter()
            .filter(|(path, edited)| self.workspace_files.get(*path) != Some(*edited))
            .map(|(path, _)| path.clone())
            .collect();
        files.sort();
        files
    }

    /// Forget everything the editor cached about the workspace.
    ///
    /// Every `Workspace*` path is relative to the thread's *current* working
    /// directory, so a cached tree or an open tab becomes a path into a
    /// different folder the moment that directory changes — and saving such a
    /// tab would write stale content over whatever same-named file happens to
    /// live there. Anything that can repoint the workspace (directory picker,
    /// thread switch, project switch, reconnect) must call this.
    ///
    /// Returns the files that had unsaved edits, so the caller can say what
    /// was dropped instead of discarding work without a word.
    pub fn reset_workspace_view(&mut self) -> Vec<std::path::PathBuf> {
        let unsaved = self.unsaved_editor_files();

        self.workspace_listings.clear();
        self.workspace_files.clear();
        self.editor_expanded.clear();
        self.editor_open.clear();
        self.editor_active = None;
        self.editor_edits.clear();
        self.editor_saving.clear();
        self.workspace_generation = self.workspace_generation.wrapping_add(1);
        unsaved
    }

    pub fn switch_thread(&mut self, target_id: u64) {
        // A different thread may run in a different directory (its own
        // override, else its project's), so the editor's view cannot carry
        // over. The caller has already resolved any unsaved changes — see
        // `PendingWorkspaceChange` — so discarding here is safe.
        self.reset_workspace_view();

        // Save current thread's messages
        if let Some(current_id) = self.foreground_thread_id
            && !self.messages.is_empty()
        {
            self.thread_messages
                .insert(current_id, self.messages.clone());
        }

        // Restore target thread's messages (or start empty).
        // The gateway will shortly send authoritative history via
        // ThreadMessages or ThreadHistoryReply — the local cache is
        // a stopgap so the sidebar highlight moves instantly.
        self.messages = self
            .thread_messages
            .get(&target_id)
            .cloned()
            .unwrap_or_default();

        // Track the switch locally instead of waiting for the gateway's
        // ThreadsUpdate round-trip: history replies arriving in between are
        // matched against this id, and the sidebar highlight moves at once.
        self.foreground_thread_id = Some(target_id);

        // Reset ALL indicators so the foreground_request_in_flight() guard
        // in hydrate_thread_messages / apply_thread_history won't block the
        // authoritative history snapshot from the gateway. The streaming
        // bubble from the previous view was already lost when we swapped
        // self.messages above; the full text arrives via the snapshot.
        self.is_processing = false;
        self.is_streaming = false;
        self.is_thinking = false;
        self.streaming_chunks = 0;
        self.streaming_bytes = 0;
    }

    /// Reset the processing/streaming indicators to idle. Does not release
    /// `streaming_thread_id` — an in-flight response keeps its owner until
    /// [`response_done`](Self::response_done) or disconnect.
    fn reset_streaming_indicators(&mut self) {
        self.is_processing = false;
        self.is_streaming = false;
        self.is_thinking = false;
        self.streaming_chunks = 0;
        self.streaming_bytes = 0;
    }
}

/// Convert a thread's persisted history, as it arrives on the wire, into the
/// transcript the chat surface renders.
///
/// Tool results (`role == "tool"`) are folded into the matching tool call on
/// the assistant turn that issued them rather than emitted as standalone
/// bubbles, so replayed history looks like the live stream did. This is the
/// single conversion for both history frames the gateway sends —
/// `ThreadMessages` and `ThreadHistoryReply` — which previously had separate
/// copies that had already drifted apart on role mapping.
pub(crate) fn ui_history_from_gateway(
    messages: Vec<protocol::types::ChatMessage>,
) -> VecDeque<ChatMessage> {
    let mut out: VecDeque<ChatMessage> = VecDeque::with_capacity(messages.len());
    for m in messages.into_iter() {
        if m.role == "tool"
            && let Some(call_id) = m.tool_call_id.as_deref()
            && let Some(prev) = out.iter_mut().rev().find(|c| {
                c.role == rustyclaw_core::types::MessageRole::Assistant
                    && c.tool_calls.iter().any(|tc| tc.id == call_id)
            })
        {
            if let Some(tc) = prev.tool_calls.iter_mut().find(|tc| tc.id == call_id) {
                tc.result = Some(m.content.clone());
                tc.is_error = false;
            }
            continue;
        }
        out.push_back(ui_message_from_gateway(m));
    }
    out
}

fn ui_message_from_gateway(message: protocol::types::ChatMessage) -> ChatMessage {
    use rustyclaw_core::ui::ToolCallInfo;

    let role = match message.role.as_str() {
        "user" => rustyclaw_core::types::MessageRole::User,
        "assistant" => rustyclaw_core::types::MessageRole::Assistant,
        "system" => rustyclaw_core::types::MessageRole::System,
        "tool" => rustyclaw_core::types::MessageRole::ToolResult,
        // Roles this build doesn't know still belong on screen; a neutral
        // system line is the honest rendering.
        _ => rustyclaw_core::types::MessageRole::System,
    };

    let mut tool_calls: Vec<ToolCallInfo> = Vec::new();
    if let Some(tcs) = message.tool_calls.as_ref().and_then(|v| v.as_array()) {
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
                duration_ms: None,
                live_status: None,
                live_output: String::new(),
            });
        }
    }

    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        content: message.display_content(),
        timestamp: chrono::Utc::now(),
        tool_calls,
        is_streaming: false,
        duration_ms: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclaw_core::types::MessageRole;

    /// Regression test: with extended thinking, the reasoning block folds
    /// when the first answer chunk arrives — the chunk must open a fresh
    /// assistant bubble *after* the block, not be dropped because the
    /// folded block sits at the tail.
    #[test]
    fn answer_text_survives_a_thinking_block() {
        let mut s = AppState::default();
        s.messages.clear();

        s.start_assistant_message(); // StreamStart
        s.start_thinking_message(); // ThinkingStart
        s.append_thinking("plan the answer");
        s.end_thinking_message(); // ThinkingEnd (first chunk imminent)
        s.append_to_current_message("Hello"); // Chunk
        s.append_to_current_message(", world");
        s.response_done();

        let roles: Vec<MessageRole> = s.messages.iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![MessageRole::Thinking, MessageRole::Assistant]);
        assert_eq!(s.messages[1].content, "Hello, world");
        assert!(!s.messages[1].is_streaming);
        assert_eq!(s.messages[0].content, "plan the answer");
        assert!(s.messages[0].duration_ms.is_some());
    }

    /// Tool calls arriving after a folded thinking block attach to the
    /// turn's assistant bubble, not to the thinking message.
    #[test]
    fn tool_calls_skip_folded_thinking_blocks() {
        let mut s = AppState::default();
        s.messages.clear();

        s.start_assistant_message();
        s.start_thinking_message();
        s.append_thinking("let me check something");
        s.end_thinking_message();
        s.add_tool_call("t1".into(), "read_file".into(), "{}".into());
        s.set_tool_result("t1", "ok".into(), false);

        assert!(s.messages[0].tool_calls.is_empty());
        let assistant = &s.messages[1];
        assert_eq!(assistant.role, MessageRole::Assistant);
        assert_eq!(assistant.tool_calls.len(), 1);
        assert!(assistant.tool_calls[0].duration_ms.is_some());
    }

    /// A thinking block that never received reasoning text disappears
    /// instead of rendering an empty shell.
    #[test]
    fn empty_thinking_blocks_are_dropped() {
        let mut s = AppState::default();
        s.messages.clear();

        s.start_assistant_message();
        s.start_thinking_message();
        s.end_thinking_message();
        s.append_to_current_message("answer");

        let roles: Vec<MessageRole> = s.messages.iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![MessageRole::Assistant]);
        assert_eq!(s.messages[0].content, "answer");
    }

    // ── Thread history loading ──────────────────────────────────────────

    fn wire(role: &str, content: &str) -> protocol::types::ChatMessage {
        protocol::types::ChatMessage::text(role, content)
    }

    /// A two-turn conversation as the gateway persists and replays it.
    fn wire_history() -> Vec<protocol::types::ChatMessage> {
        vec![wire("user", "what is it"), wire("assistant", "this is it")]
    }

    fn idle_state() -> AppState {
        let mut s = AppState::default();
        s.messages.clear();
        s
    }

    /// Persisted history keeps its roles: a replayed conversation renders as
    /// user and assistant turns, not as a wall of neutral system lines.
    #[test]
    fn replayed_history_keeps_user_and_assistant_roles() {
        let converted = ui_history_from_gateway(wire_history());
        let roles: Vec<MessageRole> = converted.iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![MessageRole::User, MessageRole::Assistant]);
    }

    /// Regression: a history snapshot that arrives before the client knows
    /// which thread is in the foreground used to be cached and never shown,
    /// leaving the transcript empty apart from local notices while the
    /// sidebar happily displayed the thread's message count.
    #[test]
    fn history_arriving_before_the_thread_list_still_renders() {
        let mut s = idle_state();
        assert_eq!(s.foreground_thread_id, None);

        s.hydrate_thread_messages(7, wire_history());

        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.foreground_thread_id, Some(7));
    }

    /// The same, for the reply to an explicit history request.
    #[test]
    fn history_reply_before_the_thread_list_still_renders() {
        let mut s = idle_state();

        s.apply_thread_history(7, ui_history_from_gateway(wire_history()));

        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.foreground_thread_id, Some(7));
    }

    /// And the other ordering: history for a thread arrives first and is
    /// cached, then the thread list names that thread as the foreground.
    /// The cached snapshot must go on screen rather than wait for another
    /// round trip that the client has no reason to make.
    #[test]
    fn thread_list_shows_history_that_already_arrived() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.apply_thread_history(7, ui_history_from_gateway(wire_history()));
        assert!(s.messages.is_empty(), "thread 7 is not on screen yet");

        s.set_foreground_thread(Some(7));

        assert_eq!(s.messages.len(), 2);
    }

    /// A snapshot for a backgrounded thread is cached, never displayed.
    #[test]
    fn history_for_another_thread_does_not_take_the_view() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);

        s.hydrate_thread_messages(2, wire_history());

        assert!(s.messages.is_empty());
        assert_eq!(s.foreground_thread_id, Some(1));
        assert_eq!(s.thread_messages.get(&2).map(VecDeque::len), Some(2));
    }

    /// The gateway's "nothing is focused" sentinel clears the view without
    /// being mistaken for a thread of its own.
    #[test]
    fn the_no_thread_sentinel_clears_without_being_adopted() {
        let mut s = idle_state();
        s.hydrate_thread_messages(7, wire_history());
        assert_eq!(s.messages.len(), 2);

        s.hydrate_thread_messages(0, Vec::new());

        assert!(s.messages.is_empty());
        assert_eq!(s.foreground_thread_id, Some(7));
        assert!(!s.thread_messages.contains_key(&0));
    }

    /// A snapshot must not replace the live view while the on-screen thread
    /// is mid-response — that would drop the streaming bubble.
    #[test]
    fn history_waits_while_the_foreground_request_is_in_flight() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(7);
        s.mark_request_started();

        s.hydrate_thread_messages(7, wire_history());

        assert!(s.messages.is_empty());
        assert!(s.is_processing);
        assert_eq!(s.thread_messages.get(&7).map(VecDeque::len), Some(2));
    }

    /// Every `Workspace*` path is relative to the *current* working directory,
    /// so a cached tree or open tab from a previous directory is not merely
    /// stale — saving it would write that content into a same-named file in
    /// the new directory. Everything must go.
    #[test]
    fn resetting_the_workspace_view_clears_every_editor_cache() {
        let mut s = AppState::default();
        let path = std::path::PathBuf::from("src/main.rs");

        s.workspace_listings
            .insert(std::path::PathBuf::new(), Vec::new());
        s.workspace_files.insert(path.clone(), "loaded".into());
        s.editor_expanded.insert(std::path::PathBuf::from("src"));
        s.editor_open.push(path.clone());
        s.editor_active = Some(path.clone());
        s.editor_edits.insert(path.clone(), "typed".into());
        s.editor_saving.insert(path.clone(), "in flight".into());

        let dropped = s.reset_workspace_view();

        assert_eq!(
            dropped,
            vec![path],
            "unsaved work is reported, not silently dropped"
        );
        assert!(s.workspace_listings.is_empty());
        assert!(s.workspace_files.is_empty());
        assert!(s.editor_expanded.is_empty());
        assert!(s.editor_open.is_empty());
        assert!(s.editor_active.is_none());
        assert!(s.editor_edits.is_empty());
        assert!(
            s.editor_saving.is_empty(),
            "an in-flight save must not reconcile into the new directory"
        );
    }

    /// A file whose edit matches what was loaded has nothing unsaved, so it is
    /// not reported as discarded — the warning should name real losses only.
    #[test]
    fn resetting_reports_only_genuinely_unsaved_files() {
        let mut s = AppState::default();
        let clean = std::path::PathBuf::from("clean.rs");
        s.workspace_files.insert(clean.clone(), "same".into());
        s.editor_edits.insert(clean, "same".into());

        assert!(s.reset_workspace_view().is_empty());
    }

    /// The prompt and the discard warning must agree about what is at stake,
    /// so both read from `unsaved_editor_files`.
    #[test]
    fn unsaved_files_lists_only_real_changes_and_is_non_destructive() {
        let mut s = AppState::default();
        let dirty = std::path::PathBuf::from("b.rs");
        let clean = std::path::PathBuf::from("a.rs");
        let fresh = std::path::PathBuf::from("c.rs");

        s.workspace_files.insert(clean.clone(), "same".into());
        s.editor_edits.insert(clean, "same".into());
        s.workspace_files.insert(dirty.clone(), "old".into());
        s.editor_edits.insert(dirty.clone(), "new".into());
        // Never loaded, but typed into: unsaved.
        s.editor_edits.insert(fresh.clone(), "typed".into());

        let mut expected = vec![dirty, fresh];
        expected.sort();
        assert_eq!(s.unsaved_editor_files(), expected);

        // Asking must not disturb anything — the user may still cancel.
        assert_eq!(s.editor_edits.len(), 3);
        assert_eq!(s.unsaved_editor_files(), expected);
    }

    /// The gateway never pushes a listing, so a reset has to be observable or
    /// the tree stays blank forever. The counter is what makes it observable.
    #[test]
    fn resetting_bumps_the_workspace_generation() {
        let mut s = AppState::default();
        let before = s.workspace_generation;
        s.reset_workspace_view();
        assert_ne!(s.workspace_generation, before);

        // Every reset is distinct, so consecutive switches each re-fetch.
        let after_one = s.workspace_generation;
        s.reset_workspace_view();
        assert_ne!(s.workspace_generation, after_one);
    }
}
