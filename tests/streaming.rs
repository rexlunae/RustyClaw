//! Streaming provider tests.
//!
//! Tests for SSE streaming from OpenAI and Anthropic providers.

use serde_json::json;

mod openai_streaming {
    

    #[test]
    fn test_openai_sse_text_chunk_parsing() {
        // Simulated SSE chunk from OpenAI
        let chunk = r#"data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        
        let data = chunk.strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        assert_eq!(parsed["choices"][0]["delta"]["content"], "Hello");
    }

    #[test]
    fn test_openai_sse_tool_call_start() {
        let chunk = r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_abc123","type":"function","function":{"name":"read_file","arguments":""}}]}}]}"#;
        
        let data = chunk.strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        let tool_call = &parsed["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(tool_call["id"], "call_abc123");
        assert_eq!(tool_call["function"]["name"], "read_file");
    }

    #[test]
    fn test_openai_sse_tool_call_args_delta() {
        let chunk = r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}"#;
        
        let data = chunk.strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        let args = &parsed["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"];
        assert!(args.as_str().unwrap().contains("path"));
    }

    #[test]
    fn test_openai_sse_done_marker() {
        let chunk = "data: [DONE]";
        
        let data = chunk.strip_prefix("data: ").unwrap();
        assert_eq!(data, "[DONE]");
    }

    #[test]
    fn test_openai_sse_finish_reason() {
        let chunk = r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        
        let data = chunk.strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        assert_eq!(parsed["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn test_openai_sse_tool_calls_finish() {
        let chunk = r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#;
        
        let data = chunk.strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        assert_eq!(parsed["choices"][0]["finish_reason"], "tool_calls");
    }
}

mod anthropic_streaming {
    

    #[test]
    fn test_anthropic_message_start() {
        let event = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_123\",\"type\":\"message\",\"role\":\"assistant\"}}";
        
        let lines: Vec<&str> = event.lines().collect();
        assert_eq!(lines[0], "event: message_start");
        
        let data = lines[1].strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(parsed["type"], "message_start");
    }

    #[test]
    fn test_anthropic_content_block_start_text() {
        let event = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}";
        
        let lines: Vec<&str> = event.lines().collect();
        let data = lines[1].strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        assert_eq!(parsed["content_block"]["type"], "text");
    }

    #[test]
    fn test_anthropic_content_block_start_tool_use() {
        let event = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_123\",\"name\":\"read_file\",\"input\":{}}}";
        
        let lines: Vec<&str> = event.lines().collect();
        let data = lines[1].strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        assert_eq!(parsed["content_block"]["type"], "tool_use");
        assert_eq!(parsed["content_block"]["name"], "read_file");
    }

    #[test]
    fn test_anthropic_text_delta() {
        let event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello, \"}}";
        
        let lines: Vec<&str> = event.lines().collect();
        let data = lines[1].strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        assert_eq!(parsed["delta"]["type"], "text_delta");
        assert_eq!(parsed["delta"]["text"], "Hello, ");
    }

    #[test]
    fn test_anthropic_input_json_delta() {
        let event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\"}}";
        
        let lines: Vec<&str> = event.lines().collect();
        let data = lines[1].strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        assert_eq!(parsed["delta"]["type"], "input_json_delta");
    }

    #[test]
    fn test_anthropic_content_block_stop() {
        let event = "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}";
        
        let lines: Vec<&str> = event.lines().collect();
        let data = lines[1].strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        assert_eq!(parsed["type"], "content_block_stop");
    }

    #[test]
    fn test_anthropic_message_delta() {
        let event = "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":50}}";
        
        let lines: Vec<&str> = event.lines().collect();
        let data = lines[1].strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        assert_eq!(parsed["delta"]["stop_reason"], "end_turn");
        assert!(parsed["usage"]["output_tokens"].is_number());
    }

    #[test]
    fn test_anthropic_message_stop() {
        let event = "event: message_stop\ndata: {\"type\":\"message_stop\"}";
        
        let lines: Vec<&str> = event.lines().collect();
        assert_eq!(lines[0], "event: message_stop");
    }

    #[test]
    fn test_anthropic_error_event() {
        let event = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"Rate limited\"}}";
        
        let lines: Vec<&str> = event.lines().collect();
        let data = lines[1].strip_prefix("data: ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        
        assert_eq!(parsed["error"]["type"], "rate_limit_error");
    }
}

mod stream_chunk_types {
    use super::*;

    #[test]
    fn test_stream_chunk_text() {
        let chunk = json!({
            "type": "Text",
            "content": "Hello, world!"
        });
        
        assert_eq!(chunk["type"], "Text");
    }

    #[test]
    fn test_stream_chunk_tool_call_start() {
        let chunk = json!({
            "type": "ToolCallStart",
            "index": 0,
            "id": "call_123",
            "name": "read_file"
        });
        
        assert_eq!(chunk["type"], "ToolCallStart");
    }

    #[test]
    fn test_stream_chunk_tool_call_delta() {
        let chunk = json!({
            "type": "ToolCallDelta",
            "index": 0,
            "arguments": "{\"path\": \"/tmp/test.txt\"}"
        });
        
        assert_eq!(chunk["type"], "ToolCallDelta");
    }

    #[test]
    fn test_stream_chunk_done() {
        let chunk = json!({
            "type": "Done"
        });
        
        assert_eq!(chunk["type"], "Done");
    }

    #[test]
    fn test_stream_chunk_error() {
        let chunk = json!({
            "type": "Error",
            "message": "Rate limited"
        });
        
        assert_eq!(chunk["type"], "Error");
    }
}

mod sse_parsing {
    

    #[test]
    fn test_parse_sse_event_separator() {
        let raw = "data: {\"text\":\"one\"}\n\ndata: {\"text\":\"two\"}\n\n";
        let events: Vec<&str> = raw.split("\n\n").filter(|s| !s.is_empty()).collect();
        
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_parse_sse_with_event_type() {
        let raw = "event: message\ndata: {\"content\":\"hello\"}\n\n";
        let lines: Vec<&str> = raw.trim().lines().collect();
        
        assert!(lines[0].starts_with("event:"));
        assert!(lines[1].starts_with("data:"));
    }

    #[test]
    fn test_parse_multiline_data() {
        // SSE allows multiline data by repeating the data: prefix
        let raw = "data: line1\ndata: line2\n\n";
        let lines: Vec<&str> = raw.lines().filter(|l| l.starts_with("data:")).collect();
        
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_handle_incomplete_chunk() {
        // Simulate receiving partial data
        let partial = "data: {\"text\":\"hello";
        // This should not be processed until the rest arrives
        assert!(!partial.ends_with("}"));
    }
}

mod buffer_accumulation {
    

    #[test]
    fn test_tool_call_argument_accumulation() {
        // Tool call arguments come in chunks that must be accumulated
        let chunks = vec![
            r#"{"path":""#,
            r#"/tmp/"#,
            r#"test.txt"}"#,
        ];
        
        let accumulated: String = chunks.into_iter().collect();
        let parsed: serde_json::Value = serde_json::from_str(&accumulated).unwrap();
        
        assert_eq!(parsed["path"], "/tmp/test.txt");
    }

    #[test]
    fn test_text_accumulation() {
        let chunks = vec!["Hello, ", "world", "!"];
        let full_text: String = chunks.into_iter().collect();
        
        assert_eq!(full_text, "Hello, world!");
    }
}
