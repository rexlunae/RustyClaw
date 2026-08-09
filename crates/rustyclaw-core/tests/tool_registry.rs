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

/// Every offered tool says what it is for.
///
/// The only part of the emitted schema that is not a literal: `type`,
/// `parameters.type` and the presence of `properties` are written by
/// `tools_openai` itself and cannot vary, so asserting them would be
/// theatre.
#[test]
fn every_offered_tool_says_what_it_is_for() {
    for (name, tool) in offered() {
        let description = tool["function"]["description"].as_str().unwrap_or("");
        assert!(
            !description.trim().is_empty(),
            "{name} reaches the model with no description — nothing tells it \
             when to use the tool"
        );
    }
}

/// Tools that genuinely take no arguments.
///
/// The list exists because "no parameters" has two causes that look
/// identical on the wire: a tool that needs none, and a tool whose arm in
/// `resolve_params` was never written — the match ends in `_ => vec![]`, so
/// a miss is silent. Everything here is a bare lister; anything else with an
/// empty schema is the second case.
const PARAMETERLESS: &[&str] = &[
    "agents_list",
    "battery_health",
    "host_info",
    "load_status",
    "mcp_list",
    "service_list",
    "subagent_list",
    "swarm_list",
    "swarm_templates",
    "triggers_list",
];

/// A tool registered but forgotten in the parameter table reaches the model
/// uncallable.
///
/// This is the check `CONTRIBUTING.md` promises, and the one the trap
/// actually needs: `resolve_params` is a match separate from registration,
/// ending in a catch-all, so a missing arm produces a tool that is offered,
/// described, and impossible to invoke with any argument. `web_extract` hit
/// it once; `todo`, `skill_curator` and `ast_grep_manage` were all sitting
/// in the same state when this test was written — two of them with their
/// parameter tables already authored and simply never wired up.
#[test]
fn no_tool_is_offered_with_an_empty_schema_by_accident() {
    let orphans: Vec<String> = offered()
        .into_iter()
        .filter(|(name, tool)| {
            !PARAMETERLESS.contains(&name.as_str())
                && tool["function"]["parameters"]["properties"]
                    .as_object()
                    .is_some_and(|p| p.is_empty())
        })
        .map(|(name, _)| name)
        .collect();

    assert!(
        orphans.is_empty(),
        "offered with no parameters at all: {orphans:?}. Either add the arm \
         to `resolve_params` in tools/schema.rs, or add the tool to \
         PARAMETERLESS here if it genuinely takes none."
    );
}

/// …and the list does not outlive the tools on it.
#[test]
fn the_parameterless_list_has_no_stale_entries() {
    let names: Vec<String> = offered().into_iter().map(|(name, _)| name).collect();
    for expected in PARAMETERLESS {
        assert!(
            names.iter().any(|n| n == expected),
            "{expected} is exempted from the schema check but is no longer \
             offered — drop it from PARAMETERLESS"
        );
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
