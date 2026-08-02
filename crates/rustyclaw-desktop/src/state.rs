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

/// The editor's view of one working directory.
///
/// Tagged with the directory it describes, which is what makes a stale path
/// *unrepresentable* rather than merely wrong. Reads for a different root come
/// back empty, and replies for a different root are ignored — so nothing has
/// to remember to clear anything. Four separate bugs came from callers
/// forgetting exactly that.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct WorkspaceView {
    /// The directory every path below is relative to. `None` before the first
    /// listing arrives.
    root: Option<std::path::PathBuf>,
    listings: HashMap<std::path::PathBuf, Vec<rustyclaw_core::gateway::WorkspaceEntryDto>>,
    files: HashMap<std::path::PathBuf, String>,
    edits: HashMap<std::path::PathBuf, String>,
    saving: HashMap<std::path::PathBuf, String>,
    expanded: HashSet<std::path::PathBuf>,
    open: Vec<std::path::PathBuf>,
    active: Option<std::path::PathBuf>,
    /// Bumped on every rebase. The gateway only sends a listing in reply to a
    /// request, so something must ask again after a move; watching a counter
    /// rather than "are the listings empty" keeps a directory that genuinely
    /// lists as empty — or one whose listing failed — from asking forever.
    generation: u64,
}

impl WorkspaceView {
    /// Incremented every time the view rebases.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The directory this view describes, if it has one yet.
    pub fn root(&self) -> Option<&std::path::Path> {
        self.root.as_deref()
    }

    /// Whether a reply tagged with `root` belongs to this view.
    ///
    /// A view with no root yet accepts the first reply and adopts its root:
    /// that is how it learns where it is.
    pub fn accepts(&self, root: &std::path::Path) -> bool {
        match &self.root {
            Some(current) => current == root,
            None => true,
        }
    }

    /// Learn where this view lives from a reply, when it did not know yet.
    /// Only ever called after [`Self::accepts`], so it cannot relabel a view
    /// that already has a root.
    pub fn adopt_root(&mut self, root: std::path::PathBuf) {
        if self.root.is_none() {
            self.root = Some(root);
        }
    }

    /// Start over in `root`, returning the files whose edits were unsaved.
    ///
    /// A no-op when the root is unchanged, so a spurious call cannot discard
    /// work — the case that produced two of the four bugs.
    pub fn rebase(&mut self, root: std::path::PathBuf) -> Vec<std::path::PathBuf> {
        if self.root.as_ref() == Some(&root) {
            return Vec::new();
        }
        let unsaved = self.unsaved();
        let generation = self.generation.wrapping_add(1);
        *self = Self {
            root: Some(root),
            generation,
            ..Self::default()
        };
        unsaved
    }

    /// Forget where we are, so the next reply re-establishes it.
    ///
    /// Used when the workspace is known to have moved but not yet to where —
    /// a thread or project switch, whose new directory only the gateway knows.
    pub fn rebase_to_current_thread(&mut self) -> Vec<std::path::PathBuf> {
        let unsaved = self.unsaved();
        let generation = self.generation.wrapping_add(1);
        *self = Self {
            generation,
            ..Self::default()
        };
        unsaved
    }

    /// Treat every unsaved edit as written.
    ///
    /// Used when the user chose Save before moving the workspace: the writes
    /// are queued and the view is about to be rebased, so waiting for the
    /// replies would only produce a "discarded" warning for work the user
    /// explicitly saved. A write that fails still surfaces its own error.
    pub fn assume_saved(&mut self) {
        for path in self.unsaved() {
            if let Some(edited) = self.edits.get(&path).cloned() {
                self.files.insert(path, edited);
            }
        }
    }

    /// Files with edits that differ from what was loaded.
    pub fn unsaved(&self) -> Vec<std::path::PathBuf> {
        let mut files: Vec<std::path::PathBuf> = self
            .edits
            .iter()
            .filter(|(path, edited)| self.files.get(*path) != Some(*edited))
            .map(|(path, _)| path.clone())
            .collect();
        files.sort();
        files
    }

    /// Directory listings, keyed by directory path relative to the root.
    pub fn listings(
        &self,
    ) -> &HashMap<std::path::PathBuf, Vec<rustyclaw_core::gateway::WorkspaceEntryDto>> {
        &self.listings
    }
    /// Loaded file contents, keyed by path relative to the root.
    pub fn files(&self) -> &HashMap<std::path::PathBuf, String> {
        &self.files
    }
    /// Typed-but-unwritten contents, keyed by path relative to the root.
    pub fn edits(&self) -> &HashMap<std::path::PathBuf, String> {
        &self.edits
    }
    /// Directories the tree currently shows expanded.
    pub fn expanded(&self) -> &HashSet<std::path::PathBuf> {
        &self.expanded
    }
    /// Open editor tabs, in display order.
    pub fn open(&self) -> &[std::path::PathBuf] {
        &self.open
    }
    /// The tab currently being edited.
    pub fn active(&self) -> Option<&std::path::Path> {
        self.active.as_deref()
    }

    /// Record a directory's entries as they arrived from the gateway.
    pub fn set_listing(
        &mut self,
        path: std::path::PathBuf,
        entries: Vec<rustyclaw_core::gateway::WorkspaceEntryDto>,
    ) {
        self.listings.insert(path, entries);
    }
    /// Record a file's contents as they arrived from the gateway.
    pub fn set_file(&mut self, path: std::path::PathBuf, content: String) {
        self.files.insert(path, content);
    }
    /// Record what the user has typed into a file.
    pub fn set_edit(&mut self, path: std::path::PathBuf, content: String) {
        self.edits.insert(path, content);
    }
    /// Remember the text being written, so the reply can reconcile it.
    pub fn begin_save(&mut self, path: std::path::PathBuf, content: String) {
        self.saving.insert(path, content);
    }
    /// Reconcile a completed save: the written text becomes the loaded text.
    pub fn finish_save(&mut self, path: &std::path::Path, ok: bool) {
        if let (Some(written), true) = (self.saving.remove(path), ok) {
            self.files.insert(path.to_path_buf(), written);
        }
    }
    /// Toggle a directory open/closed; returns true when it now needs listing.
    pub fn toggle_dir(&mut self, path: std::path::PathBuf) -> bool {
        if self.expanded.remove(&path) {
            return false;
        }
        let needs = !self.listings.contains_key(&path);
        self.expanded.insert(path);
        needs
    }
    /// Open a file in a tab and focus it.
    pub fn open_file(&mut self, path: std::path::PathBuf) {
        if !self.open.contains(&path) {
            self.open.push(path.clone());
        }
        self.active = Some(path);
    }
    /// Close a tab, keeping any unsaved text for when it is reopened.
    pub fn close_file(&mut self, path: &std::path::Path) {
        self.open.retain(|p| p != path);
        if self.active.as_deref() == Some(path) {
            self.active = self.open.last().cloned();
        }
    }
    /// Bring an already-open tab to the front.
    pub fn focus(&mut self, path: std::path::PathBuf) {
        self.active = Some(path);
    }
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
    /// Threads with a turn this client is waiting on.
    ///
    /// A set, not one id: the gateway runs a turn per thread, so sending in
    /// one conversation and then another leaves two in flight. A single slot
    /// held whichever started last — which made Stop cancel the wrong
    /// conversation, discarded the older turn's close-out so its bubble never
    /// finished, and filed its questions under the newer thread.
    pub in_flight: std::collections::HashSet<u64>,

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
    /// Tool approvals waiting for the user, oldest first. A queue, not a
    /// slot: turns run per thread, and two of them can ask at once. A second
    /// request used to overwrite the first, which was then never shown again
    /// — the gateway's waiter timed out after two minutes and read the
    /// silence as a denial of a tool the user never saw.
    pub pending_tool_approvals: std::collections::VecDeque<(String, String, String)>,

    /// Pending user prompt from the agent.
    /// Questions from the agent, each tagged with the thread whose turn
    /// asked it. One card renders at a time — the first belonging to the
    /// thread on screen — and the rest wait their turn instead of being
    /// overwritten and lost.
    pub pending_user_prompts: Vec<(Option<u64>, UserPrompt)>,

    /// The thread the pending question belongs to. Questions are per-thread,
    /// not per-window: switching to another thread hides the card, switching
    /// back restores it, so a question never holds the whole app hostage.
    /// `None` means the owning thread is unknown (the question arrived before
    /// any thread existed), in which case the card shows wherever the user is.

    /// Pending credential request (id, provider, secret_name, message).
    /// Credential requests waiting for the user, oldest first. See
    /// `pending_tool_approvals` for why this is a queue.
    pub pending_credential_requests: std::collections::VecDeque<(String, String, String, String)>,

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

    /// The editor's view of the current working directory.
    pub workspace: WorkspaceView,

    /// A workspace change waiting on the unsaved-changes prompt.
    pub pending_workspace_change: Option<PendingWorkspaceChange>,

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
            in_flight: std::collections::HashSet::new(),
            agent_name: None,
            vault_locked: false,
            needs_hatching,
            model,
            provider,
            prompt_attachments: Vec::new(),
            sidebar_collapsed: false,
            theme: Theme::default(),
            pending_tool_approvals: std::collections::VecDeque::new(),
            pending_user_prompts: Vec::new(),
            pending_credential_requests: std::collections::VecDeque::new(),
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
            workspace: WorkspaceView::default(),
            pending_workspace_change: None,
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
        if let Some(thread) = self.foreground_thread_id {
            self.in_flight.insert(thread);
        }
    }

    /// Whether live stream events (StreamStart/Chunk/Thinking/ToolCall…)
    /// target the thread currently on screen. `None` means the response
    /// thread is unknown (e.g. submitted before any thread existed) and
    /// events apply to whatever is in the foreground.
    pub fn stream_targets_foreground(&self) -> bool {
        match self.foreground_thread_id {
            Some(thread) => self.in_flight.contains(&thread),
            // Nothing focused: a turn was submitted before any thread
            // existed, and its frames apply to whatever comes into view.
            None => true,
        }
    }

    /// Whether a turn-scoped frame from `thread_id` should render into the
    /// view on screen.
    ///
    /// Routed by the frame's *own* thread rather than by a single "the turn
    /// in flight" slot. With a turn running in each of two threads, that slot
    /// holds whichever started last, so the other turn's chunks would be
    /// appended to the wrong transcript — two answers spliced into one.
    ///
    /// A frame from any other thread belongs to a turn the user is not
    /// looking at, and is dropped: that thread's transcript arrives whole via
    /// the gateway's history snapshot when its turn completes. `None` is a
    /// gateway too old to attribute its frames, and is trusted as before.
    pub fn frame_targets_view(&self, thread_id: Option<u64>) -> bool {
        match thread_id {
            Some(id) => self.foreground_thread_id == Some(id),
            None => true,
        }
    }

    /// Take the gateway at its word about which thread the turn is running
    /// in. The client's own guess at submit time can be wrong — no thread
    /// was focused and the gateway elected one, or an older client let it
    /// auto-switch — and the announcement on `StreamStart` is the first
    /// point where the two can be reconciled. Everything downstream routes
    /// off `streaming_thread_id`, so correcting it here corrects the whole
    /// turn: chunks, tool calls, the question card, the completion.
    pub fn adopt_stream_thread(&mut self, thread_id: Option<u64>) {
        let Some(id) = thread_id else { return };
        self.in_flight.insert(id);
        // Nothing was focused, so the gateway elected this thread for the
        // turn. Follow it onto the screen: otherwise every frame that
        // follows is judged to belong to a thread that isn't in view, and
        // the reply — text, tool calls, all of it — renders nowhere.
        //
        // A `ThreadsUpdate` says the same thing (electing emits a
        // `Foregrounded` event, which the connection loop turns into one)
        // and would arrive first in practice. But that is two independent
        // channels agreeing on their order, and the whole point of naming
        // the thread on the turn is not having to depend on that.
        //
        // Recorded as in flight first: with the two now agreed, the turn
        // counts as running here, and cached history won't replace the live
        // view underneath it.
        if self.foreground_thread_id.is_none() {
            self.set_foreground_thread(Some(id));
        }
    }

    /// Whether a turn-scoped frame announcing `thread_id` belongs to the
    /// response this client is tracking. An unannounced thread (`None`) is
    /// an older gateway and is trusted, as is a client that never settled
    /// on one; a mismatch is another thread's turn and must not retire this
    /// one's in-flight state.
    pub fn frame_is_for_current_turn(&self, thread_id: Option<u64>) -> bool {
        match thread_id {
            Some(announced) => self.in_flight.contains(&announced),
            // An older gateway, which cannot say. Trusted as before.
            None => true,
        }
    }

    /// Record a question from the agent, tagged with the thread whose turn
    /// asked it (the streaming thread, else whatever is in the foreground).
    pub fn set_user_prompt(&mut self, prompt: UserPrompt, asked_by: Option<u64>) {
        let owner = asked_by.or(self.foreground_thread_id);
        self.pending_user_prompts.push((owner, prompt));
    }

    /// The question to render in the chat stream: `Some` only while the
    /// thread that asked it is the one on screen.
    pub fn visible_user_prompt(&self) -> Option<UserPrompt> {
        self.pending_user_prompts
            .iter()
            .find(|(owner, _)| owner.is_none() || *owner == self.foreground_thread_id)
            .map(|(_, prompt)| prompt.clone())
    }

    /// Drop the pending question. Used when it is answered or dismissed, and
    /// when the gateway stops waiting for it (cancel, timeout, tool result).
    pub fn clear_user_prompt(&mut self) {
        self.pending_user_prompts.clear();
    }

    /// Drop the pending question if `id` identifies it. The prompt id is the
    /// `ask_user` tool call id, so the tool's result frame — which arrives
    /// whether the wait ended in an answer, a cancel, or a timeout — is what
    /// retires a card the user never touched.
    pub fn clear_user_prompt_if(&mut self, id: &str) {
        self.pending_user_prompts.retain(|(_, p)| p.id != id);
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
    }

    /// Retire the in-flight turn on Stop, returning the thread it was running
    /// in so the request can name it.
    ///
    /// Stop means the conversation on screen. With a turn running in each of
    /// several threads, that is the only one the user can have meant — naming
    /// the most recently started instead would interrupt a conversation they
    /// are not even looking at.
    pub fn stop_current_turn(&mut self) -> Option<u64> {
        let thread_id = self.foreground_thread_id;
        if let Some(thread) = thread_id {
            self.in_flight.remove(&thread);
        }
        self.finish_current_message();
        // Stop applies to a turn parked on a question too: the gateway
        // drops the wait, so the card goes with it — but only the cards
        // belonging to the turn being stopped. A question asked by a turn
        // in another thread is still live, and wiping it would leave that
        // turn waiting out its full window on an answer the user can no
        // longer give.
        self.pending_user_prompts
            .retain(|(owner, _)| owner.is_some() && *owner != thread_id);
        thread_id
    }

    /// Handle the end of a response. Finalizes the live view only when the
    /// response targeted the foreground thread; a response that completed in
    /// a backgrounded thread just releases the in-flight marker (its
    /// transcript arrives via the gateway's history snapshot).
    pub fn response_done(&mut self, thread_id: Option<u64>) {
        // Each turn retires its own. Clearing the lot would take the working
        // indicator off a conversation that is still answering.
        match thread_id {
            Some(thread) => {
                self.in_flight.remove(&thread);
            }
            None => self.in_flight.clear(),
        }
        let on_screen = thread_id.is_none() || thread_id == self.foreground_thread_id;
        if on_screen {
            self.finish_current_message();
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
                    in_flight_threads = ?self.in_flight,
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

        // A different foreground thread may run in a different directory, and
        // this is the one place that learns of the change however it happened
        // — an explicit switch, a thread being deleted or created, or the
        // gateway electing a new one. Resetting here rather than in each
        // caller is what keeps a path from being missed.
        let dropped = self.workspace.rebase_to_current_thread();
        if !dropped.is_empty() {
            let names: Vec<String> = dropped.iter().map(|p| p.display().to_string()).collect();
            self.push_notice(
                rustyclaw_core::types::MessageRole::Warning,
                format!(
                    "The working directory changed with unsaved editor changes; discarded: {}",
                    names.join(", ")
                ),
            );
        }

        // The view always follows the foreground. This used to bail out
        // while a request was in flight — written when the in-flight test
        // meant "the turn being tracked is the one on screen", where it
        // protected a live streaming bubble. The test now asks whether the
        // *incoming* thread is busy, and with turns running per thread that
        // is true for any busy destination: bailing left the outgoing
        // thread's transcript on screen under the incoming thread's
        // identity, with the incoming turn's chunks appended to it. The
        // in-flight guard's remaining job is deciding whether a *history
        // snapshot* may replace the live view, in
        // `history_should_take_the_view`.
        self.messages = thread_id
            .and_then(|id| self.thread_messages.get(&id).cloned())
            .unwrap_or_default();
        // Derived, not cleared: the gateway can move the foreground onto a
        // thread whose turn is still running, and the view must say so.
        self.derive_view_indicators();
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
    pub fn switch_thread(&mut self, target_id: u64) {
        // A different thread may run in a different directory (its own
        // override, else its project's), so the editor's view cannot carry
        // over.
        //
        // Most callers route through `PendingWorkspaceChange` and have already
        // asked the user what to do. Deleting the foreground thread does not —
        // it switches to a fallback directly — so reporting here rather than
        // trusting the caller means no path can drop work in silence, this one
        // or a future one.
        // The reset and any warning belong to `set_foreground_thread`, which
        // the gateway's reply drives — that path sees every foreground change,
        // including thread deletion, which never reaches here.
        self.workspace.rebase_to_current_thread();

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

        // The indicators describe the view, so they are derived from what is
        // actually running in the incoming thread — not blanket-cleared.
        // Clearing unconditionally re-opened the composer and hid the
        // working state of a conversation still mid-answer, which also made
        // Stop unreachable for it; leaving them set would show a phantom
        // spinner over an idle thread and block its history snapshot. The
        // streaming bubble from the previous view was already lost when we
        // swapped `self.messages` above; an in-flight thread's full text
        // arrives via the completion snapshot.
        self.derive_view_indicators();
    }

    /// Point the working indicators at the thread now in the foreground.
    ///
    /// `is_processing` doubles as the composer gate, and that is wanted: a
    /// message sent into a thread whose turn is running would displace that
    /// turn, so while one is running the way to intervene is Stop — which
    /// this is what keeps reachable. Chunk/thinking arrivals re-arm the
    /// finer-grained flags on their own.
    fn derive_view_indicators(&mut self) {
        self.is_processing = self
            .foreground_thread_id
            .is_some_and(|thread| self.in_flight.contains(&thread));
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
    use std::path::{Path, PathBuf};

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
        s.response_done(s.foreground_thread_id);

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

    /// The property that makes the whole class of "forgot to invalidate" bugs
    /// impossible: a view knows which directory it describes, so a reply for a
    /// different one is not merely stale — it is rejected.
    #[test]
    fn a_view_rejects_replies_from_another_directory() {
        let mut view = WorkspaceView::default();
        // With no root yet, the first reply establishes one.
        assert!(view.accepts(Path::new("/srv/api")));
        view.adopt_root(PathBuf::from("/srv/api"));

        assert!(view.accepts(Path::new("/srv/api")));
        assert!(
            !view.accepts(Path::new("/srv/other")),
            "a reply from a directory this view is not looking at must be refused"
        );
        // Adopting cannot relabel an established view.
        view.adopt_root(PathBuf::from("/srv/other"));
        assert_eq!(view.root(), Some(Path::new("/srv/api")));
    }

    /// Rebasing to the same directory is a no-op, so a spurious call — the
    /// shape of two earlier bugs — cannot throw work away.
    #[test]
    fn rebasing_to_the_same_directory_keeps_everything() {
        let mut view = WorkspaceView::default();
        view.adopt_root(PathBuf::from("/srv/api"));
        view.set_file(PathBuf::from("a.rs"), "loaded".into());
        view.set_edit(PathBuf::from("a.rs"), "typed".into());

        assert!(view.rebase(PathBuf::from("/srv/api")).is_empty());
        assert_eq!(view.unsaved(), vec![PathBuf::from("a.rs")]);
    }

    /// Moving to a different directory drops everything and reports the loss.
    #[test]
    fn rebasing_elsewhere_reports_and_clears() {
        let mut view = WorkspaceView::default();
        view.adopt_root(PathBuf::from("/srv/api"));
        view.set_file(PathBuf::from("a.rs"), "loaded".into());
        view.set_edit(PathBuf::from("a.rs"), "typed".into());
        view.open_file(PathBuf::from("a.rs"));

        assert_eq!(
            view.rebase(PathBuf::from("/srv/new")),
            vec![PathBuf::from("a.rs")]
        );
        assert_eq!(view.root(), Some(Path::new("/srv/new")));
        assert!(view.open().is_empty());
        assert!(view.files().is_empty());
        assert!(view.unsaved().is_empty());
    }

    /// A save reconciles by promoting the written text; a failed one leaves
    /// the edit alone so the work can be retried.
    #[test]
    fn finishing_a_save_promotes_only_on_success() {
        let mut view = WorkspaceView::default();
        let path = PathBuf::from("a.rs");
        view.set_file(path.clone(), "old".into());
        view.set_edit(path.clone(), "new".into());

        view.begin_save(path.clone(), "new".into());
        view.finish_save(&path, true);
        assert!(
            view.unsaved().is_empty(),
            "a saved file is no longer unsaved"
        );

        view.set_edit(path.clone(), "newer".into());
        view.begin_save(path.clone(), "newer".into());
        view.finish_save(&path, false);
        assert_eq!(view.unsaved(), vec![path], "a failed save keeps the work");
    }

    /// Closing a tab keeps its text: closing is not a way to discard work.
    #[test]
    fn closing_a_tab_keeps_unsaved_text() {
        let mut view = WorkspaceView::default();
        let path = PathBuf::from("a.rs");
        view.set_file(path.clone(), "old".into());
        view.set_edit(path.clone(), "new".into());
        view.open_file(path.clone());

        view.close_file(&path);
        assert!(view.open().is_empty());
        assert_eq!(view.unsaved(), vec![path]);
    }

    /// Expanding fetches once; collapsing and reopening reuses the listing.
    #[test]
    fn toggling_a_directory_fetches_only_the_first_time() {
        let mut view = WorkspaceView::default();
        let dir = PathBuf::from("src");

        assert!(view.toggle_dir(dir.clone()), "first expand needs a listing");
        view.set_listing(dir.clone(), Vec::new());
        assert!(!view.toggle_dir(dir.clone()), "collapsing fetches nothing");
        assert!(!view.toggle_dir(dir), "reopening reuses what arrived");
    }

    /// Deleting or creating a thread never routes through the guarded switch —
    /// the gateway just reports a new foreground. That path has to invalidate
    /// the view too, or the editor keeps showing the previous directory.
    #[test]
    fn a_foreground_change_from_the_gateway_rebases_the_view() {
        let mut s = AppState {
            foreground_thread_id: Some(1),
            ..AppState::default()
        };
        s.workspace.adopt_root(PathBuf::from("/srv/api"));
        s.workspace.set_file(PathBuf::from("a.rs"), "loaded".into());
        s.workspace.open_file(PathBuf::from("a.rs"));

        s.set_foreground_thread(Some(2));

        assert!(s.workspace.open().is_empty(), "stale tabs must not survive");
        assert!(s.workspace.files().is_empty());
        assert_eq!(
            s.workspace.root(),
            None,
            "the next reply re-establishes where we are"
        );
    }

    /// Choosing Save must not then be told the work was discarded.
    #[test]
    fn assuming_saved_silences_the_discard_warning() {
        let mut view = WorkspaceView::default();
        view.set_file(PathBuf::from("a.rs"), "old".into());
        view.set_edit(PathBuf::from("a.rs"), "new".into());
        assert_eq!(view.unsaved(), vec![PathBuf::from("a.rs")]);

        view.assume_saved();
        assert!(
            view.unsaved().is_empty(),
            "queued saves must not be reported as discarded"
        );
        assert!(view.rebase_to_current_thread().is_empty());
    }

    fn question(id: &str) -> UserPrompt {
        UserPrompt {
            id: id.to_string(),
            title: "Which way?".to_string(),
            description: None,
            prompt_type: rustyclaw_core::user_prompt_types::PromptType::TextInput {
                placeholder: None,
                default: None,
            },
        }
    }

    /// A question belongs to the thread that asked it: switching away hides
    /// the card, switching back brings it out. Without this the question
    /// follows the user everywhere, which is what made an inline card behave
    /// like a modal.
    #[test]
    fn a_question_is_scoped_to_the_thread_that_asked_it() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.mark_request_started();

        s.set_user_prompt(question("call-1"), s.foreground_thread_id);
        assert_eq!(
            s.visible_user_prompt().map(|p| p.id).as_deref(),
            Some("call-1")
        );

        s.switch_thread(2);
        assert!(
            s.visible_user_prompt().is_none(),
            "another thread's question must not be on screen"
        );
        assert!(
            !s.pending_user_prompts.is_empty(),
            "switching away parks the question, it does not answer it"
        );

        s.switch_thread(1);
        assert_eq!(
            s.visible_user_prompt().map(|p| p.id).as_deref(),
            Some("call-1")
        );
    }

    /// A question cannot outlive the connection that asked it: the turn is
    /// gone, no tool result will ever retire the card, and an answer would
    /// go into a closed socket.
    #[test]
    fn a_dropped_connection_retires_the_question() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.mark_request_started();
        s.set_user_prompt(question("call-1"), s.foreground_thread_id);

        // What the Disconnected handler does.
        s.is_processing = false;
        s.is_streaming = false;
        s.is_thinking = false;
        s.in_flight.clear();
        s.clear_user_prompt();

        assert!(s.visible_user_prompt().is_none());
        assert!(s.pending_user_prompts.is_empty());
    }

    /// The `ask_user` tool result means the gateway stopped waiting —
    /// answered, cancelled or timed out. The card goes with it, even if the
    /// user is looking at a different thread.
    #[test]
    fn a_tool_result_retires_the_question_it_belongs_to() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.set_user_prompt(question("call-1"), s.foreground_thread_id);

        s.clear_user_prompt_if("some-other-call");
        assert!(!s.pending_user_prompts.is_empty());

        s.clear_user_prompt_if("call-1");
        assert!(s.pending_user_prompts.is_empty());
    }

    /// The gateway has the last word on which thread a turn is running in.
    ///
    /// The client guesses at submit time from its own foreground, and that
    /// guess can be wrong — nothing was focused and the gateway elected a
    /// thread, or an older client let it auto-switch. `StreamStart` names
    /// the real one, and everything downstream routes off it.
    #[test]
    fn the_announced_thread_corrects_the_clients_guess() {
        let mut s = idle_state();
        s.foreground_thread_id = None;
        s.mark_request_started();
        assert!(s.in_flight.is_empty());

        s.adopt_stream_thread(Some(7));
        assert!(s.in_flight.contains(&7));

        // An older gateway announces nothing; nothing is added.
        s.adopt_stream_thread(None);
        assert_eq!(s.in_flight.len(), 1);
    }

    /// A thread elected for the turn comes onto the screen with it.
    ///
    /// Sending with nothing focused makes the gateway elect a thread and
    /// announce it. Adopting it as the turn's thread but leaving the view
    /// pointed at nothing puts the two permanently out of step: every
    /// frame after `StreamStart` reads as belonging to a backgrounded
    /// thread and the whole reply is dropped on the floor.
    #[test]
    fn an_elected_thread_comes_into_view_with_its_turn() {
        let mut s = idle_state();
        s.foreground_thread_id = None;
        s.mark_request_started();

        s.adopt_stream_thread(Some(7));

        assert_eq!(s.foreground_thread_id, Some(7));
        assert!(
            s.stream_targets_foreground(),
            "the turn must render into the view, not past it"
        );
    }

    /// Electing does not yank the user out of the thread they are reading.
    ///
    /// The announcement settles where the *turn* runs. A client that
    /// already knows its foreground has said where it is looking, and a
    /// turn running elsewhere — one started before the user switched away —
    /// must stay in the background where it belongs.
    #[test]
    fn an_announcement_does_not_move_a_view_that_is_already_placed() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(3);
        s.mark_request_started();

        s.adopt_stream_thread(Some(7));

        assert_eq!(
            s.foreground_thread_id,
            Some(3),
            "a turn opening elsewhere must not move the view"
        );
        assert!(s.in_flight.contains(&7), "the other turn is tracked too");
    }

    /// Each turn's completion retires only its own.
    ///
    /// Sending in one conversation and then another leaves two turns in
    /// flight. A single "the turn" slot held whichever started last, so the
    /// first turn's close-out was discarded as somebody else's — its bubble
    /// never finished and the working indicator never cleared. And the slot
    /// decided what Stop named, so Stop cancelled the newer conversation
    /// while the user was looking at the older one.
    #[test]
    fn two_turns_in_flight_retire_independently() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.mark_request_started();

        // The user switches to thread 2 and sends there; thread 1 is still
        // answering.
        s.switch_thread(2);
        s.mark_request_started();
        assert!(s.in_flight.contains(&1) && s.in_flight.contains(&2));

        // Thread 1 finishes. It is a real turn of this client's, so its
        // close-out is its own — but it must not finish the bubble on screen,
        // which belongs to thread 2.
        assert!(s.frame_is_for_current_turn(Some(1)));
        s.response_done(Some(1));
        assert!(!s.in_flight.contains(&1));
        assert!(
            s.is_processing,
            "thread 2 is still answering; the indicator stays"
        );

        // Thread 2 finishes, and that is the one on screen.
        s.response_done(Some(2));
        assert!(s.in_flight.is_empty());
        assert!(!s.is_processing);
    }

    /// Stop interrupts the conversation on screen.
    ///
    /// Named from the most recently started turn, it would cancel a
    /// conversation the user is not even looking at while the one they meant
    /// keeps running.
    #[test]
    fn stop_targets_the_conversation_on_screen() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.mark_request_started();
        s.switch_thread(2);
        s.mark_request_started();

        // Back to the older conversation, which is still answering.
        s.switch_thread(1);
        assert_eq!(
            s.stop_current_turn(),
            Some(1),
            "Stop must name what the user is reading, not the newest turn"
        );
        assert!(!s.in_flight.contains(&1));
        assert!(
            s.in_flight.contains(&2),
            "the other conversation keeps running"
        );
    }

    /// Two conversations' questions each render in their own thread.
    ///
    /// A second question used to overwrite the first, which was never shown
    /// again — its five-minute wait expired on an answer the user was never
    /// offered. Now each card waits with its thread, and answering one
    /// retires only it.
    #[test]
    fn two_threads_questions_each_render_in_their_own() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.set_user_prompt(question("call-1"), Some(1));
        s.set_user_prompt(question("call-2"), Some(2));

        assert_eq!(
            s.visible_user_prompt().map(|p| p.id).as_deref(),
            Some("call-1")
        );
        s.switch_thread(2);
        assert_eq!(
            s.visible_user_prompt().map(|p| p.id).as_deref(),
            Some("call-2")
        );

        // Answering thread 2's question leaves thread 1's waiting.
        s.clear_user_prompt_if("call-2");
        assert!(s.visible_user_prompt().is_none());
        s.switch_thread(1);
        assert_eq!(
            s.visible_user_prompt().map(|p| p.id).as_deref(),
            Some("call-1")
        );
    }

    /// A gateway-driven foreground move always swaps the transcript.
    ///
    /// The old early return, guarding a live streaming bubble, fired for
    /// any busy destination once the in-flight test became per-thread —
    /// leaving the outgoing thread's messages on screen under the incoming
    /// thread's identity, with the incoming turn's chunks appended to them.
    #[test]
    fn a_foreground_move_onto_a_busy_thread_swaps_the_view() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.add_user_message("thread one's words".to_string());
        // Thread 2 is mid-answer with cached history.
        s.in_flight.insert(2);
        s.thread_messages.insert(
            2,
            VecDeque::from(vec![ChatMessage::user("thread two's words")]),
        );

        s.set_foreground_thread(Some(2));

        assert!(
            s.messages
                .back()
                .is_some_and(|m| m.content.contains("thread two")),
            "the view must show the thread it claims to"
        );
        assert!(
            s.is_processing,
            "the incoming thread is still answering and must say so"
        );
    }

    /// Answering one question leaves the others queued.
    ///
    /// The respond handler used the clear-everything path, so answering the
    /// card on screen silently discarded every other thread's question with
    /// it. Retirement is by call id.
    #[test]
    fn answering_one_question_leaves_the_others_queued() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.set_user_prompt(question("call-1"), Some(1));
        s.set_user_prompt(question("call-2"), Some(2));

        // What on_prompt_respond does for the visible card.
        s.clear_user_prompt_if("call-1");

        s.switch_thread(2);
        assert_eq!(
            s.visible_user_prompt().map(|p| p.id).as_deref(),
            Some("call-2"),
            "the other thread's question must still be waiting"
        );
    }

    /// Stop leaves another conversation's question alone.
    ///
    /// Questions are thread-scoped; Stop is too. Wiping the card
    /// unconditionally left the asking turn waiting out its five-minute
    /// window on an answer the user could no longer give.
    #[test]
    fn stop_leaves_another_threads_question_alone() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.mark_request_started();
        s.set_user_prompt(question("call-1"), Some(1));

        // The user switches to thread 2, starts a turn there, and stops it.
        s.switch_thread(2);
        s.mark_request_started();
        s.stop_current_turn();

        assert!(
            !s.pending_user_prompts.is_empty(),
            "thread 1's question is still waiting for its answer"
        );

        // Stopping thread 1 itself does retire its question.
        s.switch_thread(1);
        s.stop_current_turn();
        assert!(s.pending_user_prompts.is_empty());
    }

    /// Returning to a conversation that is still answering shows it working.
    ///
    /// The indicators are derived from `in_flight` on every switch, in both
    /// directions. Blanket-clearing re-opened the composer over a running
    /// turn — a send there displaces the turn, so the honest offer is Stop,
    /// and Stop is gated on the same flag. Leaving them set showed a phantom
    /// spinner over idle threads.
    #[test]
    fn returning_to_a_busy_thread_restores_its_working_state() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.mark_request_started();

        s.switch_thread(2);
        assert!(!s.is_processing, "thread 2 has no turn running");

        s.switch_thread(1);
        assert!(
            s.is_processing,
            "thread 1 is still answering; the working state and Stop come back"
        );

        s.response_done(Some(1));
        assert!(!s.is_processing);
    }

    /// Stop names the turn it is stopping.
    ///
    /// The id was read *after* `finish_current_message` had already cleared
    /// it, so every Stop went out unnamed — which the gateway honours only
    /// while exactly one turn is running and ignores when several are. A Stop
    /// button that silently does nothing. Reading and retiring are one
    /// operation now, so the order cannot be got wrong at a call site.
    #[test]
    fn stopping_a_turn_reports_the_thread_it_was_in() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(4);
        s.mark_request_started();
        s.set_user_prompt(question("call-1"), s.foreground_thread_id);

        let stopped = s.stop_current_turn();

        assert_eq!(stopped, Some(4), "the Stop must name its turn");
        assert!(!s.is_processing);
        assert!(!s.in_flight.contains(&4));
        assert!(
            s.pending_user_prompts.is_empty(),
            "a turn parked on a question loses the card with it"
        );
    }

    /// Only the turn being tracked can end it.
    ///
    /// A close-out for another thread — a message refused because its thread
    /// had gone, a turn from another client on the same agent — used to be
    /// indistinguishable from this turn's own, and retiring on it took the
    /// Stop button and the working indicator away while the model was still
    /// running.
    #[test]
    fn a_close_out_for_another_thread_leaves_this_turn_running() {
        let mut s = idle_state();
        s.foreground_thread_id = Some(1);
        s.mark_request_started();

        assert!(!s.frame_is_for_current_turn(Some(2)));
        assert!(s.frame_is_for_current_turn(Some(1)));
        // Unannounced: an older gateway, and trusted as before.
        assert!(s.frame_is_for_current_turn(None));

        // What the ResponseDone handler does when it matches.
        if s.frame_is_for_current_turn(Some(1)) {
            s.response_done(s.foreground_thread_id);
        }
        assert!(!s.is_processing);
    }
}
