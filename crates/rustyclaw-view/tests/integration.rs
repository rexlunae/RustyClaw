//! Integration tests for `rustyclaw-view` component data types.
//!
//! These tests verify that `From` impls correctly convert canonical
//! domain models into component-view types without data loss.

use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::{ChatMessage, ThreadInfo, ToolCallInfo};
use rustyclaw_view::{MessageBubbleData, SidebarItemData, ToolCallData};

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
