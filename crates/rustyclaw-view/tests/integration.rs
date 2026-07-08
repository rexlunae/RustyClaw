//! Integration tests for `rustyclaw-view` component data types.
//!
//! These tests verify that `From` impls correctly convert canonical
//! domain models into component-view types without data loss.

use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::{ChatMessage, ThreadInfo, ToolCallInfo};
use rustyclaw_view::{
    ApiKeyDialogData, AuthDialogData, CredentialRequestData, MessageBubbleData, ModelSelectorData,
    PairingStep, ProviderOptionData, ProviderSelectorData, SidebarItemData, StatusBarData,
    ToolApprovalData, ToolCallData, VaultUnlockData,
};

// ── MessageBubbleData ────────────────────────────────────────────────

#[test]
fn from_chat_message_preserves_fields() {
    let msg = ChatMessage {
        id: "msg-1".into(),
        role: MessageRole::User,
        content: "Hello, world!".into(),
        timestamp: chrono::Utc::now(),
        tool_calls: vec![],
        is_streaming: false,
        duration_ms: None,
    };

    let data = MessageBubbleData::from_chat_message(&msg, Some("Luthen".into()));

    assert_eq!(data.role, MessageRole::User);
    assert_eq!(data.content, "Hello, world!");
    assert!(!data.is_streaming);
    assert_eq!(data.agent_name, Some("Luthen".into()));
    assert!(!data.has_details);
}

#[test]
fn from_chat_message_handles_streaming() {
    let msg = ChatMessage {
        id: "msg-2".into(),
        role: MessageRole::Assistant,
        content: "Thinking...".into(),
        timestamp: chrono::Utc::now(),
        tool_calls: vec![],
        is_streaming: true,
        duration_ms: None,
    };

    let data = MessageBubbleData::from_chat_message(&msg, None);

    assert!(data.is_streaming);
    assert_eq!(data.agent_name, None);
}

#[test]
fn from_chat_message_maps_tool_call_role_correctly() {
    let msg = ChatMessage {
        id: "msg-3".into(),
        role: MessageRole::ToolCall,
        content: "web_search".into(),
        timestamp: chrono::Utc::now(),
        tool_calls: vec![],
        is_streaming: false,
        duration_ms: None,
    };

    let data = MessageBubbleData::from_chat_message(&msg, None);
    assert_eq!(data.role, MessageRole::ToolCall);
}

// ── ToolCallData ─────────────────────────────────────────────────────

#[test]
fn from_tool_call_info_preserves_fields() {
    let tc = ToolCallInfo {
        id: "tc-1".into(),
        name: "web_search".into(),
        arguments: r#"{"query": "test"}"#.into(),
        result: Some("results".into()),
        is_error: false,
        collapsed: true,
        duration_ms: None,
        live_status: None,
        live_output: String::new(),
    };

    let data = ToolCallData::from(&tc);

    assert_eq!(data.id, "tc-1");
    assert_eq!(data.name, "web_search");
    assert!(data.arguments.contains("query"));
    assert!(data.arguments.contains("test"));
    assert_eq!(data.result, Some("results".into()));
    assert!(!data.is_error);
    assert!(data.collapsed);
}

#[test]
fn from_tool_call_info_pretty_prints_json() {
    let tc = ToolCallInfo {
        id: "tc-2".into(),
        name: "write_file".into(),
        arguments: r#"{"path":"/tmp/test.txt","content":"hello"}"#.into(),
        result: None,
        is_error: false,
        collapsed: true,
        duration_ms: None,
        live_status: None,
        live_output: String::new(),
    };

    let data = ToolCallData::from(&tc);

    // Should contain pretty-printed JSON with newlines
    assert!(data.arguments.contains('\n'));
    assert!(data.arguments.contains("path"));
    assert!(data.arguments.contains("content"));
    assert!(data.result.is_none());
}

#[test]
fn from_tool_call_info_preserves_error_flag() {
    let tc = ToolCallInfo {
        id: "tc-3".into(),
        name: "execute_command".into(),
        arguments: "{}".into(),
        result: Some("Error: command not found".into()),
        is_error: true,
        collapsed: false,
        duration_ms: None,
        live_status: None,
        live_output: String::new(),
    };

    let data = ToolCallData::from(&tc);

    assert!(data.is_error);
    assert!(!data.collapsed);
    assert_eq!(data.result, Some("Error: command not found".into()));
}

// ── SidebarItemData ──────────────────────────────────────────────────

#[test]
fn from_thread_info_preserves_fields() {
    let ti = ThreadInfo {
        id: 42,
        project_id: 1,
        label: Some("Research".into()),
        description: Some("Epstein files".into()),
        status: "active".into(),
        is_foreground: true,
        message_count: 128,
    };

    let data = SidebarItemData::from(&ti);

    assert_eq!(data.id, 42);
    assert_eq!(data.project_id, 1);
    assert_eq!(data.label, Some("Research".into()));
    assert_eq!(data.description, Some("Epstein files".into()));
    assert_eq!(data.status, "active");
    assert!(data.is_foreground);
    assert_eq!(data.message_count, 128);
}

#[test]
fn from_thread_info_no_label() {
    let ti = ThreadInfo {
        id: 7,
        project_id: 0,
        label: None,
        description: None,
        status: "idle".into(),
        is_foreground: false,
        message_count: 0,
    };

    let data = SidebarItemData::from(&ti);

    assert_eq!(data.id, 7);
    assert!(data.label.is_none());
    assert!(!data.is_foreground);
    assert_eq!(data.message_count, 0);
}

// ── Edge cases ──────────────────────────────────────────────────────

#[test]
fn empty_content_handled() {
    let msg = ChatMessage {
        id: "empty".into(),
        role: MessageRole::System,
        content: String::new(),
        timestamp: chrono::Utc::now(),
        tool_calls: vec![],
        is_streaming: false,
        duration_ms: None,
    };

    let data = MessageBubbleData::from_chat_message(&msg, None);
    assert!(data.content.is_empty());
}

#[test]
fn invalid_json_args_are_passed_through() {
    let tc = ToolCallInfo {
        id: "bad-json".into(),
        name: "weird_tool".into(),
        arguments: "not valid json at all".into(),
        result: None,
        is_error: false,
        collapsed: false,
        duration_ms: None,
        live_status: None,
        live_output: String::new(),
    };

    let data = ToolCallData::from(&tc);
    // pretty_print_json falls back to raw string
    assert_eq!(data.arguments, "not valid json at all");
}

// ── Direct construction (no ChatMessage) ────────────────────────────

#[test]
fn direct_construction_no_timestamp() {
    let data = MessageBubbleData {
        collapsed: false,
        role: MessageRole::Assistant,
        content: "Hello".into(),
        timestamp: None,
        is_streaming: false,
        agent_name: Some("Luthen".into()),
        has_details: true,
        duration_ms: None,
    };

    assert_eq!(data.role, MessageRole::Assistant);
    assert_eq!(data.content, "Hello");
    assert!(data.timestamp.is_none());
    assert!(data.agent_name.is_some());
    assert!(data.has_details);
}

#[test]
fn from_chat_message_preserves_timestamp() {
    let now = chrono::Utc::now();
    let msg = ChatMessage {
        id: "ts-test".into(),
        role: MessageRole::User,
        content: "time check".into(),
        timestamp: now,
        tool_calls: vec![],
        is_streaming: false,
        duration_ms: None,
    };

    let data = MessageBubbleData::from_chat_message(&msg, None);
    assert_eq!(data.timestamp, Some(now));
}

// ── MessageBubbleData shared display methods ────────────────────────

#[test]
fn display_name_for_user() {
    let data = MessageBubbleData {
        role: MessageRole::User,
        content: "hi".into(),
        ..Default::default()
    };
    assert_eq!(data.display_name(), "You");
}

#[test]
fn display_name_for_assistant_with_agent_name() {
    let data = MessageBubbleData {
        role: MessageRole::Assistant,
        content: "hello".into(),
        agent_name: Some("Nemik".into()),
        ..Default::default()
    };
    assert_eq!(data.display_name(), "Nemik");
}

#[test]
fn display_name_for_assistant_without_agent_name() {
    let data = MessageBubbleData {
        role: MessageRole::Assistant,
        content: "hello".into(),
        agent_name: None,
        ..Default::default()
    };
    assert_eq!(data.display_name(), "Assistant");
}

#[test]
fn display_name_for_assistant_with_empty_agent_name() {
    let data = MessageBubbleData {
        role: MessageRole::Assistant,
        content: "hello".into(),
        agent_name: Some("".into()),
        ..Default::default()
    };
    assert_eq!(data.display_name(), "Assistant");
}

#[test]
fn should_render_markdown_for_assistant_not_streaming() {
    let data = MessageBubbleData {
        role: MessageRole::Assistant,
        content: "# hi".into(),
        is_streaming: false,
        ..Default::default()
    };
    assert!(data.should_render_markdown());
}

#[test]
fn should_not_render_markdown_for_assistant_streaming() {
    let data = MessageBubbleData {
        role: MessageRole::Assistant,
        content: "# hi".into(),
        is_streaming: true,
        ..Default::default()
    };
    assert!(!data.should_render_markdown());
}

#[test]
fn should_not_render_markdown_for_user() {
    let data = MessageBubbleData {
        role: MessageRole::User,
        ..Default::default()
    };
    assert!(!data.should_render_markdown());
}

#[test]
fn display_content_truncates_long_thinking() {
    let data = MessageBubbleData {
        role: MessageRole::Thinking,
        content: "a".repeat(200),
        ..Default::default()
    };
    let displayed = data.display_content();
    assert!(displayed.len() < 200);
    assert!(displayed.ends_with("…"));
}

#[test]
fn display_content_passes_short_thinking_through() {
    let data = MessageBubbleData {
        role: MessageRole::Thinking,
        content: "short".into(),
        ..Default::default()
    };
    assert_eq!(data.display_content(), "short");
}

#[test]
fn display_content_passes_other_roles_through() {
    let data = MessageBubbleData {
        role: MessageRole::Assistant,
        content: "# long markdown content".into(),
        ..Default::default()
    };
    // display_content only truncates Thinking; assistant passes through
    assert_eq!(data.display_content(), "# long markdown content");
}

// ── SidebarItemData shared display methods ──────────────────────────

#[test]
fn sidebar_display_label_falls_back_to_session_number() {
    let item = SidebarItemData {
        id: 7,
        label: None,
        ..Default::default()
    };
    assert_eq!(item.display_label(), "Session #7");
}

#[test]
fn sidebar_display_label_uses_user_label() {
    let item = SidebarItemData {
        id: 7,
        label: Some("World build".into()),
        ..Default::default()
    };
    assert_eq!(item.display_label(), "World build");
}

#[test]
fn sidebar_truncated_label_keeps_short_labels() {
    let item = SidebarItemData {
        label: Some("Hi".into()),
        ..Default::default()
    };
    assert_eq!(item.truncated_label(10), "Hi");
}

#[test]
fn sidebar_truncated_label_shortens_long_labels() {
    let item = SidebarItemData {
        label: Some("A very long label here".into()),
        ..Default::default()
    };
    let truncated = item.truncated_label(10);
    assert!(truncated.len() < 22);
    assert!(truncated.ends_with("…"));
}

// ── ToolApprovalData shared display methods ─────────────────────────

#[test]
fn tool_approval_summary() {
    let ta = ToolApprovalData {
        id: "tc1".into(),
        name: "web_search".into(),
        arguments: r#"{"q":"hello"}"#.into(),
        selected_allow: true,
    };
    assert_eq!(ta.summary(), "🔧 web_search");
}

#[test]
fn tool_approval_arguments_preview_truncates() {
    let ta = ToolApprovalData {
        id: "tc1".into(),
        name: "test".into(),
        arguments: "a".repeat(500),
        selected_allow: true,
    };
    let preview = ta.arguments_preview(50, 5);
    assert!(preview.len() <= 55);
}

// ── AuthDialogData shared display methods ───────────────────────────

#[test]
fn auth_is_complete_with_6_digits() {
    let ad = AuthDialogData {
        code: "123456".into(),
        ..Default::default()
    };
    assert!(ad.is_complete());
}

#[test]
fn auth_masked_code_shows_entered_and_remaining() {
    let ad = AuthDialogData {
        code: "12".into(),
        ..Default::default()
    };
    let masked = ad.masked_code();
    assert_eq!(masked, "● ● ○ ○ ○ ○");
}

// ── CredentialRequestData shared display methods ────────────────────

#[test]
fn credential_summary() {
    let cr = CredentialRequestData {
        provider: "anthropic".into(),
        secret_name: "API key".into(),
        message: "need key".into(),
        input_len: 0,
    };
    assert_eq!(cr.summary(), "🔑 API key — anthropic");
}

#[test]
fn credential_masked_input() {
    let cr = CredentialRequestData {
        input_len: 3,
        ..default_credential()
    };
    assert_eq!(cr.masked_input(), "•••");
    assert!(cr.has_input());
}

#[test]
fn provider_selector_selected_provider() {
    let data = ProviderSelectorData {
        providers: vec![
            ProviderOptionData {
                id: "anthropic".into(),
                display_name: "Anthropic".into(),
                auth_hint: "apikey".into(),
            },
            ProviderOptionData {
                id: "github".into(),
                display_name: "GitHub Copilot".into(),
                auth_hint: "deviceflow".into(),
            },
        ],
        cursor: 1,
    };

    let selected = data.selected().expect("selected provider");
    assert_eq!(selected.id, "github");
    assert_eq!(selected.auth_badge(), " 🔗");
}

#[test]
fn api_key_masked_input_uses_width() {
    let data = ApiKeyDialogData {
        input_len: 4,
        ..Default::default()
    };

    assert_eq!(data.masked_input(8), "••••····");
}

#[test]
fn model_selector_visible_window_and_scroll_hint() {
    let data = ModelSelectorData {
        models: (0..20).map(|i| format!("model-{i}")).collect(),
        cursor: 10,
        ..Default::default()
    };

    assert_eq!(data.visible_window(5), (8, 13));
    assert_eq!(data.scroll_hint(5), "  (11/20)");
    assert_eq!(data.selected_model(), Some("model-10"));
}

fn default_credential() -> CredentialRequestData {
    CredentialRequestData {
        provider: String::new(),
        secret_name: String::new(),
        message: String::new(),
        input_len: 0,
    }
}

// ── StatusBarData shared display methods ────────────────────────────

#[test]
fn status_bar_connection_labels() {
    use rustyclaw_core::ui::ConnectionStatus;
    let mut sb = StatusBarData::default();
    assert_eq!(sb.connection_label(), "Disconnected");
    sb.connection = ConnectionStatus::Connected;
    assert_eq!(sb.connection_label(), "Connected");
    sb.connection = ConnectionStatus::Error("broken".into());
    assert_eq!(sb.connection_label(), "Error");
}

#[test]
fn status_bar_static_methods() {
    use ConnectionStatus::*;
    use rustyclaw_core::ui::ConnectionStatus;

    assert_eq!(
        StatusBarData::connection_label_static(&Disconnected),
        "Disconnected"
    );
    assert_eq!(
        StatusBarData::connection_class_static(&Connecting),
        "is-info"
    );
    assert_eq!(
        StatusBarData::connection_class_static(&Connected),
        "is-success"
    );
    assert_eq!(
        StatusBarData::connection_class_static(&Authenticated),
        "is-success"
    );
    assert_eq!(
        StatusBarData::connection_class_static(&Error("x".into())),
        "is-danger"
    );
    assert_eq!(
        StatusBarData::connection_label_static(&Authenticating),
        "Authenticating…"
    );
    assert!(StatusBarData::connection_error_static(&Connecting).is_none());
    assert_eq!(
        StatusBarData::connection_error_static(&Error("boom".into())),
        Some("boom")
    );
}

#[test]
fn status_bar_connection_class() {
    use rustyclaw_core::ui::ConnectionStatus;
    let mut sb = StatusBarData::default();
    assert_eq!(sb.connection_class(), "is-warn");
    sb.connection = ConnectionStatus::Connected;
    assert_eq!(sb.connection_class(), "is-success");
    sb.connection = ConnectionStatus::Error("err".into());
    assert_eq!(sb.connection_class(), "is-danger");
}

#[test]
fn status_bar_model_display() {
    let mut sb = StatusBarData::default();
    assert_eq!(sb.model_display(), "(no model)");
    sb.provider = Some("openrouter".into());
    assert_eq!(sb.model_display(), "openrouter");
    sb.model = Some("gpt-4o".into());
    assert_eq!(sb.model_display(), "openrouter · gpt-4o");
    sb.provider = None;
    assert_eq!(sb.model_display(), "gpt-4o");
}

#[test]
fn status_bar_is_connected() {
    use rustyclaw_core::ui::ConnectionStatus;
    let mut sb = StatusBarData::default();
    assert!(!sb.is_connected());
    sb.connection = ConnectionStatus::Connected;
    assert!(sb.is_connected());
    sb.connection = ConnectionStatus::Authenticated;
    assert!(sb.is_connected());
}

#[test]
fn status_bar_connection_error() {
    use rustyclaw_core::ui::ConnectionStatus;
    let mut sb = StatusBarData::default();
    assert!(sb.connection_error().is_none());
    sb.connection = ConnectionStatus::Error("fail".into());
    assert_eq!(sb.connection_error(), Some("fail"));
}

// ── VaultUnlockData shared display methods ──────────────────────────

#[test]
fn vault_unlock_masked_password() {
    let vu = VaultUnlockData {
        password_len: 5,
        ..Default::default()
    };
    assert_eq!(vu.masked_password(), "•••••");
    assert!(vu.has_input());
}

// ── PairingStep shared display methods ──────────────────────────────

#[test]
fn pairing_step_labels() {
    assert_eq!(PairingStep::ShowKey.label(), "Show public key");
    assert_eq!(PairingStep::Complete.label(), "Pairing complete");
    assert!(PairingStep::Connecting.is_progress());
    assert!(PairingStep::Complete.is_complete());
}

// ── Agent self-explanation: compact tool summaries + reasoning ──────

#[test]
fn compact_action_describes_common_tools() {
    let tc = |name: &str, args: &str| ToolCallData {
        name: name.into(),
        arguments: args.into(),
        ..Default::default()
    };
    assert_eq!(
        tc(
            "read_file",
            r#"{"path":"src/main.rs","start_line":10,"end_line":80}"#
        )
        .compact_action(),
        "read src/main.rs:10–80"
    );
    assert_eq!(
        tc("execute_command", r#"{"command":"cargo test --workspace"}"#).compact_action(),
        "$ cargo test --workspace"
    );
    assert_eq!(
        tc("search_files", r#"{"pattern":"TODO","path":"crates"}"#).compact_action(),
        "search \"TODO\" in crates"
    );
    assert_eq!(
        tc("web_search", r#"{"query":"rust iocraft"}"#).compact_action(),
        "web search \"rust iocraft\""
    );
    // Unknown tool falls back to name + first string argument.
    assert_eq!(
        tc("custom_tool", r#"{"target":"alpha"}"#).compact_action(),
        "custom_tool alpha"
    );
    // Unparseable arguments fall back to just the name.
    assert_eq!(tc("mystery", "not json").compact_action(), "mystery");
}

#[test]
fn compact_action_truncates_long_commands() {
    let long = format!("{{\"command\":\"echo {}\"}}", "x".repeat(200));
    let tc = ToolCallData {
        name: "execute_command".into(),
        arguments: long,
        ..Default::default()
    };
    let action = tc.compact_action();
    assert!(action.starts_with("$ echo "));
    assert!(action.ends_with('…'));
    assert!(action.chars().count() < 70);
}

#[test]
fn result_gist_summarises_outcomes() {
    let mut tc = ToolCallData {
        name: "execute_command".into(),
        arguments: r#"{"command":"false"}"#.into(),
        result: Some("some output\nExit code: 2".into()),
        ..Default::default()
    };
    assert_eq!(tc.result_gist().as_deref(), Some("exit 2"));

    tc.name = "search_files".into();
    tc.result = Some("a.rs:1:x\nb.rs:2:y\n".into());
    assert_eq!(tc.result_gist().as_deref(), Some("2 matches"));

    tc.name = "read_file".into();
    tc.result = Some("l1\nl2\nl3".into());
    assert_eq!(tc.result_gist().as_deref(), Some("3 lines"));

    // Errors always gist to their first line.
    tc.is_error = true;
    tc.result = Some("No such file or directory\ndetails follow".into());
    assert_eq!(
        tc.result_gist().as_deref(),
        Some("No such file or directory…")
    );

    // Still running → no gist.
    tc.result = None;
    assert!(tc.result_gist().is_none());
}

#[test]
fn process_and_backgrounded_calls_read_clearly() {
    // The `process` tool describes its background-session action.
    let poll = ToolCallData {
        name: "process".into(),
        arguments: r#"{"action":"poll","sessionId":"a1b2c3d4e5f6"}"#.into(),
        ..Default::default()
    };
    assert_eq!(poll.compact_action(), "process poll a1b2c3d4");

    let list = ToolCallData {
        name: "process".into(),
        arguments: r#"{"action":"list"}"#.into(),
        ..Default::default()
    };
    assert_eq!(list.compact_action(), "process list");

    // A backgrounded command returns a JSON status blob — its gist reads
    // "backgrounded <session>", not a misleading "1 lines".
    let bg = ToolCallData {
        name: "execute_command".into(),
        arguments: r#"{"command":"sleep 120"}"#.into(),
        result: Some(r#"{"status":"running","sessionId":"a1b2c3d4e5f6","message":"…"}"#.into()),
        ..Default::default()
    };
    assert_eq!(bg.result_gist().as_deref(), Some("backgrounded a1b2c3d4"));

    // A `process` poll reporting an exited session summarises the status.
    let exited = ToolCallData {
        name: "process".into(),
        arguments: r#"{"action":"poll","sessionId":"a1b2c3d4"}"#.into(),
        result: Some(r#"{"status":"exited (0)","sessionId":"a1b2c3d4"}"#.into()),
        ..Default::default()
    };
    assert_eq!(exited.result_gist().as_deref(), Some("exited (0) a1b2c3d4"));

    // A normal (non-backgrounded) command still gists to its line count.
    let normal = ToolCallData {
        name: "execute_command".into(),
        arguments: r#"{"command":"ls"}"#.into(),
        result: Some("a.txt\nb.txt\nc.txt".into()),
        ..Default::default()
    };
    assert_eq!(normal.result_gist().as_deref(), Some("3 lines"));

    // A command that merely emits JSON with a "status" (no sessionId) — e.g.
    // a health check returning {"status":"ok"} — must NOT be mistaken for a
    // backgrounded session; it gists by line count like any other output.
    let health = ToolCallData {
        name: "execute_command".into(),
        arguments: r#"{"command":"curl .../health"}"#.into(),
        result: Some(r#"{"status":"ok"}"#.into()),
        ..Default::default()
    };
    assert_eq!(health.result_gist().as_deref(), Some("1 lines"));
}

#[test]
fn format_duration_is_human() {
    use rustyclaw_view::format_duration_ms;
    assert_eq!(format_duration_ms(432), "0.4s");
    assert_eq!(format_duration_ms(9_400), "9.4s");
    assert_eq!(format_duration_ms(42_000), "42s");
    assert_eq!(format_duration_ms(125_000), "2m 05s");
}

#[test]
fn thinking_bubble_collapses_to_one_line_gist() {
    let mut data = MessageBubbleData {
        role: MessageRole::Thinking,
        content: "First I check the config.\nThen I look at the registry.".into(),
        collapsed: true,
        ..Default::default()
    };
    assert!(data.is_collapsible());
    let rendered = data.content_for_render();
    assert!(rendered.starts_with("First I check the config.…"));
    assert!(rendered.contains("Ctrl+E"));

    // Expanded → the full reasoning text, untruncated.
    data.collapsed = false;
    assert_eq!(
        data.content_for_render(),
        "First I check the config.\nThen I look at the registry."
    );
}

#[test]
fn thinking_summary_carries_duration() {
    let mut data = MessageBubbleData {
        role: MessageRole::Thinking,
        content: "reasoning".into(),
        ..Default::default()
    };
    assert_eq!(data.thinking_summary(), "Thought");
    data.duration_ms = Some(4_200);
    assert_eq!(data.thinking_summary(), "Thought for 4.2s");
    assert_eq!(data.header_label(), "Thought for 4.2s");
    data.duration_ms = None;
    data.is_streaming = true;
    assert_eq!(data.thinking_summary(), "Thinking…");
}

#[test]
fn thinking_truncation_is_char_boundary_safe() {
    // 120 two-byte characters: byte 120 falls mid-character, which used
    // to panic in display_content_truncated's byte slicing.
    let data = MessageBubbleData {
        role: MessageRole::Thinking,
        content: "é".repeat(200),
        ..Default::default()
    };
    let shown = data.display_content();
    assert!(shown.ends_with('…'));
    assert_eq!(shown.chars().count(), 121); // 120 chars + ellipsis

    // The collapsed gist path handles multi-byte input too.
    let collapsed = MessageBubbleData {
        collapsed: true,
        ..data
    };
    let gist = collapsed.content_for_render();
    assert!(gist.contains('…'));
}

#[test]
fn conversions_propagate_duration() {
    let mut msg = ChatMessage::start_thinking();
    msg.duration_ms = Some(1_500);
    let bubble = MessageBubbleData::from_chat_message(&msg, None);
    assert_eq!(bubble.duration_ms, Some(1_500));

    let tc = ToolCallInfo {
        id: "tc".into(),
        name: "read_file".into(),
        arguments: "{}".into(),
        result: Some("ok".into()),
        is_error: false,
        collapsed: true,
        duration_ms: Some(400),
        live_status: None,
        live_output: String::new(),
    };
    assert_eq!(ToolCallData::from(&tc).duration_ms, Some(400));
}

// ── Live process status (long-running tool display) ─────────────────

#[test]
fn live_status_line_formats_process_stats() {
    let mut tc = ToolCallData {
        name: "execute_command".into(),
        arguments: r#"{"command":"cargo build"}"#.into(),
        live_status: Some(rustyclaw_core::ui::ToolLiveStatus {
            elapsed_ms: 12_000,
            pid: Some(4242),
            cpu_percent: Some(87.4),
            memory_bytes: Some(145 * 1024 * 1024),
            state: Some("running".into()),
            message: None,
        }),
        ..Default::default()
    };

    let line = tc.live_status_line().expect("running call has a status");
    assert_eq!(line, "⏳ 12s · running · cpu 87% · mem 145 MB · pid 4242");
    assert!(tc.is_controllable());
    assert!(!tc.is_process_paused());

    // A paused process reads as paused and flips the control hint.
    tc.live_status.as_mut().unwrap().state = Some("paused".into());
    assert!(tc.is_process_paused());

    // Once the result arrives the live status line disappears.
    tc.result = Some("done".into());
    assert_eq!(tc.live_status_line(), None);
    assert!(!tc.is_controllable());
}

#[test]
fn live_status_line_without_process_shows_elapsed_only() {
    let tc = ToolCallData {
        name: "web_search".into(),
        live_status: Some(rustyclaw_core::ui::ToolLiveStatus {
            elapsed_ms: 3_400,
            pid: None,
            cpu_percent: None,
            memory_bytes: None,
            state: None,
            message: None,
        }),
        ..Default::default()
    };
    assert_eq!(tc.live_status_line().as_deref(), Some("⏳ 3.4s"));
    // No PID → nothing to pause or kill.
    assert!(!tc.is_controllable());
}

#[test]
fn live_status_line_includes_tool_message() {
    let tc = ToolCallData {
        name: "engine_pull".into(),
        live_status: Some(rustyclaw_core::ui::ToolLiveStatus {
            elapsed_ms: 60_000,
            pid: None,
            cpu_percent: None,
            memory_bytes: None,
            state: None,
            message: Some("downloading model 42%".into()),
        }),
        ..Default::default()
    };
    assert_eq!(
        tc.live_status_line().as_deref(),
        Some("⏳ 60s · downloading model 42%")
    );
}

#[test]
fn set_tool_live_status_targets_running_call_and_clears_on_result() {
    use rustyclaw_view::DisplayMessageData;

    let mut msg = DisplayMessageData::assistant("");
    msg.add_tool_call("tc-1".into(), "execute_command".into(), "{}".into());

    let status = rustyclaw_core::ui::ToolLiveStatus {
        elapsed_ms: 2_000,
        pid: Some(7),
        cpu_percent: None,
        memory_bytes: None,
        state: Some("sleeping".into()),
        message: None,
    };
    assert!(msg.set_tool_live_status("tc-1", status.clone()));
    assert!(msg.tool_calls[0].live_status.is_some());

    // Finishing the call clears the live status.
    msg.set_tool_result("tc-1", "ok".into(), false, Some(2_100));
    assert!(msg.tool_calls[0].live_status.is_none());

    // A finished call no longer accepts status updates.
    assert!(!msg.set_tool_live_status("tc-1", status));
}

#[test]
fn format_bytes_is_human() {
    assert_eq!(rustyclaw_view::format_bytes(512 * 1024), "512 KB");
    assert_eq!(rustyclaw_view::format_bytes(145 * 1024 * 1024), "145 MB");
    assert_eq!(
        rustyclaw_view::format_bytes(2 * 1024 * 1024 * 1024 + 300 * 1024 * 1024),
        "2.3 GB"
    );
}
