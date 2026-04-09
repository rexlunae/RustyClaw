use rustyclaw_core::gateway::ChatMessage;
use rustyclaw_core::model_runtime::{
    converted_text_for_test, converted_tool_count_for_test, normalize_finish_reason_for_test,
};
use rustyclaw_core::models::ModelRegistry;

#[test]
fn adapter_converts_structured_assistant_text() {
    let msg = ChatMessage::text(
        "assistant",
        r#"{"role":"assistant","content":"Hello from rig","tool_calls":[]}"#,
    );

    assert_eq!(converted_text_for_test(&msg).unwrap(), "Hello from rig");
}

#[test]
fn adapter_extracts_tool_calls_from_structured_messages() {
    let msg = ChatMessage::text(
        "assistant",
        r#"{"role":"assistant","content":"Checking","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"/tmp/demo\"}"}}]}"#,
    );

    assert_eq!(converted_tool_count_for_test(&msg).unwrap(), 1);
}

#[test]
fn adapter_normalizes_finish_reasons() {
    assert_eq!(normalize_finish_reason_for_test(true), "stop");
    assert_eq!(normalize_finish_reason_for_test(false), "tool_calls");
}

#[test]
fn registry_defaults_are_built_from_provider_catalog_models() {
    let registry = ModelRegistry::with_defaults();

    assert!(registry.get("anthropic/claude-opus-4-20250514").is_some());
    assert!(registry.get("openai/gpt-4.1").is_some());
    assert!(registry.get("google/gemini-2.5-pro").is_some());
}
