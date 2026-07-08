//! Application state management.
//!
//! Shared UI types (`ChatMessage`, `ToolCallInfo`, `ThreadInfo`,
//! `ConnectionStatus`) live in [`rustyclaw_core::ui`]. This module
//! adds desktop-specific wrappers: the Dioxus-friendly `AppState` struct
//! and the `Theme` enum.

use std::collections::{HashMap, VecDeque};

use rustyclaw_core::gateway::protocol;
use rustyclaw_core::ui::{ChatMessage, ConnectionStatus, ThreadInfo};
use rustyclaw_core::user_prompt_types::UserPrompt;
use rustyclaw_view::{PromptAttachment, SecretsDialogData};
use rustyclaw_view::{chrono, uuid};

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

    /// Whether the right sidebar (file browser) is visible.
    pub right_sidebar_visible: bool,

    /// File browser data for the right sidebar.
    pub file_browser: rustyclaw_view::FileBrowserData,

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
            right_sidebar_visible: true,
            file_browser: working_directory
                .as_deref()
                .map(rustyclaw_view::FileBrowserData::load)
                .unwrap_or_default(),
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

    /// Replace the cached messages for a thread with an authoritative
    /// history from the gateway. If the thread is currently in the
    /// foreground, also refresh the live view.
    pub fn apply_thread_history(&mut self, thread_id: u64, messages: VecDeque<ChatMessage>) {
        self.thread_messages.insert(thread_id, messages.clone());
        if self.foreground_thread_id == Some(thread_id) && !self.foreground_request_in_flight() {
            self.messages = messages;
            self.reset_streaming_indicators();
        }
    }

    /// Replace a thread's messages with canonical history from the gateway.
    pub fn hydrate_thread_messages(
        &mut self,
        thread_id: u64,
        messages: Vec<protocol::types::ChatMessage>,
    ) {
        let hydrated: VecDeque<ChatMessage> =
            messages.into_iter().map(ui_message_from_gateway).collect();
        self.thread_messages.insert(thread_id, hydrated.clone());
        if (self.foreground_thread_id == Some(thread_id) || thread_id == 0)
            && !self.foreground_request_in_flight()
        {
            self.messages = hydrated;
            self.reset_streaming_indicators();
        }
    }

    /// Switch to a different thread, saving current messages and
    /// restoring the target thread's history.
    pub fn switch_thread(&mut self, target_id: u64) {
        // Save current thread's messages
        if let Some(current_id) = self.foreground_thread_id
            && !self.messages.is_empty()
        {
            self.thread_messages
                .insert(current_id, self.messages.clone());
        }

        // Restore target thread's messages (or start empty)
        self.messages = self
            .thread_messages
            .get(&target_id)
            .cloned()
            .unwrap_or_default();

        // Track the switch locally instead of waiting for the gateway's
        // ThreadsUpdate round-trip: history replies arriving in between are
        // matched against this id, and the sidebar highlight moves at once.
        self.foreground_thread_id = Some(target_id);

        self.reset_streaming_indicators();
        // Switching back to the thread whose response is still running:
        // surface the busy indicator again (the streamed bubble was lost
        // with the view; the full text arrives in the completion snapshot).
        self.is_processing = self.streaming_thread_id == Some(target_id);
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

fn ui_message_from_gateway(message: protocol::types::ChatMessage) -> ChatMessage {
    let role = match message.role.as_str() {
        "user" => rustyclaw_core::types::MessageRole::User,
        "assistant" => rustyclaw_core::types::MessageRole::Assistant,
        "system" => rustyclaw_core::types::MessageRole::System,
        "tool" => rustyclaw_core::types::MessageRole::ToolResult,
        _ => rustyclaw_core::types::MessageRole::Info,
    };

    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        content: message.display_content(),
        timestamp: chrono::Utc::now(),
        tool_calls: Vec::new(),
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
}
