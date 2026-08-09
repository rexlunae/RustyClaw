//! What the tool registry actually hands the model.
//!
//! Replaces the root `tests/tool_execution.rs`, which had 49 tests and one
//! real assertion: the other 48 built a `json!` literal and checked that the
//! fields they had just written were of the type they had just written. They
//! could not have failed for any change to RustyClaw.
//!
//! The one that meant something checked that `sessions_kill` reached the
//! model with a usable schema. Parameters are resolved from a match separate
//! from registration, so a tool can be registered and still be offered with
//! nothing in it — registered, visible, and uncallable. That check is kept
//! here and widened to every tool, because the failure it guards against is
//! not specific to one.

use serde_json::Value;

/// Every tool the provider is offered, keyed by name.
fn offered() -> Vec<(String, Value)> {
    rustyclaw_core::tools::tools_openai()
        .into_iter()
        .map(|tool| {
            let name = tool["function"]["name"]
                .as_str()
                .expect("every offered tool must be named")
                .to_string();
            (name, tool)
        })
        .collect()
}

#[test]
fn the_registry_offers_something() {
    let tools = offered();
    assert!(
        tools.len() > 20,
        "only {} tools reached the model — the registry is probably not \
         being built",
        tools.len()
    );
}

/// A schema the provider cannot read is a tool the model cannot call, and
/// nothing at registration time forces one to exist.
#[test]
fn every_offered_tool_carries_a_usable_schema() {
    for (name, tool) in offered() {
        assert_eq!(
            tool["type"], "function",
            "{name} is not offered as a function"
        );
        let description = tool["function"]["description"].as_str().unwrap_or("");
        assert!(
            !description.trim().is_empty(),
            "{name} reaches the model with no description — nothing tells it \
             when to use the tool"
        );

        let params = &tool["function"]["parameters"];
        assert_eq!(
            params["type"], "object",
            "{name}'s parameters are not an object schema"
        );
        assert!(
            params["properties"].is_object(),
            "{name} has no properties object; providers reject the schema"
        );
    }
}

/// A required parameter that is not declared makes the tool uncallable: the
/// provider has no shape to fill in, and rejects or omits the call.
#[test]
fn no_tool_requires_a_parameter_it_does_not_declare() {
    for (name, tool) in offered() {
        let params = &tool["function"]["parameters"];
        let declared = params["properties"]
            .as_object()
            .expect("checked by every_offered_tool_carries_a_usable_schema");
        let Some(required) = params["required"].as_array() else {
            continue;
        };
        for entry in required {
            let key = entry.as_str().unwrap_or_default();
            assert!(
                declared.contains_key(key),
                "{name} requires '{key}' but never declares it"
            );
        }
    }
}

/// The case that prompted the original check: a tool taking either of two
/// parameters must declare both and require neither.
#[test]
fn sessions_kill_accepts_a_key_or_a_label() {
    let tools = offered();
    let (_, kill) = tools
        .iter()
        .find(|(name, _)| name == "sessions_kill")
        .expect("sessions_kill must be offered to the model");

    let properties = &kill["function"]["parameters"]["properties"];
    assert!(properties["sessionKey"].is_object());
    assert!(properties["label"].is_object());
    assert_eq!(
        kill["function"]["parameters"]["required"]
            .as_array()
            .map(|r| r.len()),
        Some(0),
        "neither is required on its own — the tool accepts one or the other"
    );
}
