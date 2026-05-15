//! Integration tests for `rustyclaw-view` component data types.
//!
//! These tests verify that `From` impls correctly convert canonical
//! domain models into component-view types without data loss.

use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::{ChatMessage, ThreadInfo, ToolCallInfo};
use rustyclaw_view::{AuthDialogData, CredentialRequestData, MessageBubbleData, PairingStep, SidebarItemData, StatusBarData, ToolApprovalData, ToolCallData, VaultUnlockData};

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
        label: Some("Research".into()),
        description: Some("Epstein files".into()),
        status: "active".into(),
        is_foreground: true,
        message_count: 128,
    };

    let data = SidebarItemData::from(&ti);

    assert_eq!(data.id, 42);
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
    };

    let data = ToolCallData::from(&tc);
    // pretty_print_json falls back to raw string
    assert_eq!(data.arguments, "not valid json at all");
}

// ── Direct construction (no ChatMessage) ────────────────────────────

#[test]
fn direct_construction_no_timestamp() {
    let data = MessageBubbleData {
        role: MessageRole::Assistant,
        content: "Hello".into(),
        timestamp: None,
        is_streaming: false,
        agent_name: Some("Luthen".into()),
        has_details: true,
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
    use rustyclaw_core::ui::ConnectionStatus;
    use ConnectionStatus::*;

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
