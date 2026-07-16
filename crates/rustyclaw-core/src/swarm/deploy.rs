//! Swarm materialization and routing on top of the agent registry.
//!
//! Creating a swarm turns each [`SwarmAgent`](super::SwarmAgent) spec into a
//! persistent registered agent (`<swarm>-<member>`) whose system prompt
//! carries the member's instructions, the swarm's shared instructions, the
//! roster, and the communication flows. Deleting a swarm removes only the
//! agents it created (checked via manifest provenance), so an agent that
//! was adopted or renamed by hand is never destroyed by accident.
//!
//! Task routing spawns sub-agent sessions *under the member's registry
//! agent id* with a deterministic label (`swarm:<name>:<member>`), so
//! repeated sends reuse the member's active session and everything shows
//! up in `sessions_list` alongside ordinary agent work.

use anyhow::{Context, Result, bail};

use super::config::{AgentRole, FlowKind, SwarmAgent, SwarmConfig};
use super::store::{SwarmMemberBinding, SwarmRecord, SwarmStore};
use crate::agents::{AgentRegistry, sanitize_agent_id};
use crate::sessions::{SessionStatus, session_manager};

/// Provenance marker written into member agents' `created_by` field.
///
/// Deletion only removes agents whose manifest carries this exact marker,
/// so hand-made agents that merely share the naming scheme are safe.
pub fn swarm_created_by(swarm_name: &str) -> String {
    format!("swarm:{swarm_name}")
}

/// Session label used for a member's swarm session.
fn member_session_label(swarm_name: &str, member_id: &str) -> String {
    format!("swarm:{swarm_name}:{member_id}")
}

/// Registry agent id for a member.
fn member_agent_id(swarm_name: &str, member_id: &str) -> String {
    sanitize_agent_id(&format!("{swarm_name}-{member_id}"))
}

// ── Creation ────────────────────────────────────────────────────────────────

/// Create a swarm: validate the config, materialize every member as a
/// registered agent, and persist the record. On any failure the agents
/// created so far are rolled back, so a swarm either fully exists or not
/// at all.
pub fn create_swarm(
    store: &SwarmStore,
    registry: &AgentRegistry,
    mut config: SwarmConfig,
) -> Result<SwarmRecord> {
    let name = sanitize_agent_id(&config.name);
    if name.is_empty() {
        bail!("Could not derive a swarm name from '{}'", config.name);
    }
    config.name = name.clone();
    config.validate()?;

    if store.exists(&name) {
        bail!("Swarm '{}' already exists", name);
    }

    // Pre-compute bindings and refuse to trample existing agents before
    // creating anything.
    let bindings: Vec<SwarmMemberBinding> = config
        .agents
        .iter()
        .map(|a| SwarmMemberBinding {
            member_id: a.id.clone(),
            agent_id: member_agent_id(&name, &a.id),
        })
        .collect();
    for binding in &bindings {
        if registry.exists(&binding.agent_id) {
            bail!(
                "Agent '{}' already exists — delete it first or pick another swarm name",
                binding.agent_id
            );
        }
    }

    let created_by = swarm_created_by(&name);
    let mut created: Vec<String> = Vec::new();
    let rollback = |created: &[String]| {
        for id in created {
            let _ = registry.delete(id);
        }
    };

    for (member, binding) in config.agents.iter().zip(&bindings) {
        let prompt = compose_member_prompt(&config, member, &bindings);
        if let Err(e) = registry.create(
            Some(&binding.agent_id),
            &member.name,
            Some(&member.description),
            Some(&prompt),
            Some(&created_by),
        ) {
            rollback(&created);
            return Err(e).with_context(|| {
                format!(
                    "Failed to create agent '{}' for swarm '{}'",
                    binding.agent_id, name
                )
            });
        }
        created.push(binding.agent_id.clone());
    }

    let record = SwarmRecord {
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs()),
        members: bindings,
        config,
    };
    if let Err(e) = store.save(&record) {
        rollback(&created);
        return Err(e).context("Failed to persist swarm record");
    }
    Ok(record)
}

/// Build a member's system prompt from its instructions plus swarm-wide
/// context (shared instructions, roster with registry ids, and flows).
fn compose_member_prompt(
    config: &SwarmConfig,
    member: &SwarmAgent,
    bindings: &[SwarmMemberBinding],
) -> String {
    let agent_id_of = |member_id: &str| -> String {
        bindings
            .iter()
            .find(|b| b.member_id == member_id)
            .map(|b| b.agent_id.clone())
            .unwrap_or_else(|| member_id.to_string())
    };

    let mut prompt = String::new();
    prompt.push_str(&format!(
        "You are '{}' ({}), a member of the '{}' swarm.\n\n",
        member.name, member.role, config.name
    ));
    prompt.push_str(member.instructions.trim());
    prompt.push('\n');

    if !config.shared_instructions.trim().is_empty() {
        prompt.push_str("\n## Swarm-wide instructions\n");
        prompt.push_str(config.shared_instructions.trim());
        prompt.push('\n');
    }

    prompt.push_str("\n## Swarm roster\n");
    prompt.push_str(
        "Fellow members, by registry agent id (usable with sessions_spawn / agents_list):\n",
    );
    for other in &config.agents {
        if other.id == member.id {
            continue;
        }
        prompt.push_str(&format!(
            "- {}: {} ({}) — {}\n",
            agent_id_of(&other.id),
            other.name,
            other.role,
            other.description
        ));
    }

    let delegates: Vec<&str> = config
        .flows
        .iter()
        .filter(|f| f.from == member.id && f.kind == FlowKind::SendMessage)
        .map(|f| f.to.as_str())
        .collect();
    if !delegates.is_empty() {
        prompt.push_str("\nYou may delegate independent sub-tasks in parallel (SendMessage) to: ");
        prompt.push_str(
            &delegates
                .iter()
                .map(|id| agent_id_of(id))
                .collect::<Vec<_>>()
                .join(", "),
        );
        prompt.push('\n');
    }
    let handoffs: Vec<&str> = config
        .flows
        .iter()
        .filter(|f| f.from == member.id && f.kind == FlowKind::Handoff)
        .map(|f| f.to.as_str())
        .collect();
    if !handoffs.is_empty() {
        prompt.push_str("You may hand the conversation off entirely (Handoff) to: ");
        prompt.push_str(
            &handoffs
                .iter()
                .map(|id| agent_id_of(id))
                .collect::<Vec<_>>()
                .join(", "),
        );
        prompt.push('\n');
    }

    if !member.tools.is_empty() {
        prompt.push_str("\n## Preferred tools\n");
        prompt.push_str(&member.tools.join(", "));
        prompt.push('\n');
    }

    prompt
}

// ── Deletion / teardown ─────────────────────────────────────────────────────

/// What [`delete_swarm`] actually did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteReport {
    /// Registry agents removed (provenance matched).
    pub agents_deleted: Vec<String>,
    /// Registry agents left in place (kept on request, foreign provenance,
    /// or already gone).
    pub agents_kept: Vec<String>,
    /// Swarm sessions marked completed.
    pub sessions_completed: usize,
}

/// Delete a swarm: complete its sessions, remove the member agents it
/// created (unless `keep_agents`), and drop the record.
pub fn delete_swarm(
    store: &SwarmStore,
    registry: &AgentRegistry,
    name: &str,
    keep_agents: bool,
) -> Result<DeleteReport> {
    let record = store
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Swarm '{}' not found", name))?;

    let mut report = DeleteReport {
        sessions_completed: complete_member_sessions(&record),
        ..DeleteReport::default()
    };

    let created_by = swarm_created_by(name);
    for binding in &record.members {
        let ours = registry
            .manifest(&binding.agent_id)
            .and_then(|m| m.created_by)
            .is_some_and(|by| by == created_by);
        if !keep_agents && ours && registry.delete(&binding.agent_id).is_ok() {
            report.agents_deleted.push(binding.agent_id.clone());
        } else {
            report.agents_kept.push(binding.agent_id.clone());
        }
    }

    store.delete(name)?;
    Ok(report)
}

/// Complete all active swarm sessions for a swarm's members without
/// touching the agents or the record. Returns how many were completed.
pub fn stop_swarm_sessions(store: &SwarmStore, name: &str) -> Result<usize> {
    let record = store
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Swarm '{}' not found", name))?;
    Ok(complete_member_sessions(&record))
}

fn complete_member_sessions(record: &SwarmRecord) -> usize {
    let Ok(mut mgr) = session_manager().lock() else {
        return 0;
    };
    let mut completed = 0;
    for binding in &record.members {
        let label = member_session_label(&record.config.name, &binding.member_id);
        let key = mgr
            .get_by_label(&label)
            .filter(|s| s.status == SessionStatus::Active)
            .map(|s| s.key.clone());
        if let Some(key) = key {
            if mgr.complete_session(&key).is_ok() {
                completed += 1;
            }
        }
    }
    completed
}

// ── Routing ─────────────────────────────────────────────────────────────────

/// Where a routed message ended up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteResult {
    /// Member id the message was routed to.
    pub member_id: String,
    /// Display name of the member.
    pub member_name: String,
    /// Registry agent id backing the member.
    pub agent_id: String,
    /// Session key the message landed in.
    pub session_key: String,
    /// Whether an existing active session was reused.
    pub reused: bool,
}

/// Route a message to a swarm member (the orchestrator when `member` is
/// `None`). Reuses the member's active swarm session when one exists,
/// otherwise spawns a fresh sub-agent session under the member's registry
/// agent id.
pub fn send_to_member(
    store: &SwarmStore,
    registry: &AgentRegistry,
    name: &str,
    member: Option<&str>,
    message: &str,
) -> Result<RouteResult> {
    let record = store
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Swarm '{}' not found", name))?;

    let spec = match member {
        // Accept either the member id or the full registry agent id.
        Some(target) => record
            .config
            .agents
            .iter()
            .find(|a| a.id == target || record.agent_id_for(&a.id) == Some(target))
            .ok_or_else(|| {
                let ids: Vec<&str> = record.config.agents.iter().map(|a| a.id.as_str()).collect();
                anyhow::anyhow!(
                    "Agent '{}' not found in swarm '{}'. Available: {}",
                    target,
                    name,
                    ids.join(", ")
                )
            })?,
        None => record
            .config
            .orchestrator()
            .ok_or_else(|| anyhow::anyhow!("Swarm '{}' has no agents", name))?,
    };
    let agent_id = record
        .agent_id_for(&spec.id)
        .unwrap_or_default()
        .to_string();

    if !registry.exists(&agent_id) {
        bail!(
            "Agent '{}' for swarm member '{}' is missing from the registry — \
             was it deleted? Recreate the swarm with swarm_create.",
            agent_id,
            spec.id
        );
    }

    let label = member_session_label(name, &spec.id);
    let mut mgr = session_manager()
        .lock()
        .map_err(|_| anyhow::anyhow!("Failed to acquire session manager lock"))?;

    let existing = mgr
        .get_by_label(&label)
        .filter(|s| s.status == SessionStatus::Active)
        .map(|s| s.key.clone());

    let (session_key, reused) = match existing {
        Some(key) => {
            mgr.send_message(&key, message)?;
            (key, true)
        }
        None => {
            // The member's instructions live in its agent manifest, so the
            // task only needs the message plus a routing preamble.
            let task = format!("[Swarm {} → {}]\n\n{}", name, spec.name, message);
            (
                mgr.spawn_subagent(&agent_id, &task, Some(label), None),
                false,
            )
        }
    };

    Ok(RouteResult {
        member_id: spec.id.clone(),
        member_name: spec.name.clone(),
        agent_id,
        session_key,
        reused,
    })
}

// ── Status ──────────────────────────────────────────────────────────────────

/// Derived per-member state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberStatus {
    pub member_id: String,
    pub agent_id: String,
    pub name: String,
    pub role: AgentRole,
    pub description: String,
    /// Whether the backing registry agent currently exists.
    pub registered: bool,
    /// Whether the member has an active swarm session.
    pub session_active: bool,
}

/// Compute derived status for every member of a swarm.
pub fn member_statuses(record: &SwarmRecord, registry: &AgentRegistry) -> Vec<MemberStatus> {
    let session_active = |member_id: &str| {
        session_manager()
            .lock()
            .ok()
            .and_then(|mgr| {
                mgr.get_by_label(&member_session_label(&record.config.name, member_id))
                    .map(|s| s.status == SessionStatus::Active)
            })
            .unwrap_or(false)
    };

    record
        .config
        .agents
        .iter()
        .map(|spec| {
            let agent_id = record
                .agent_id_for(&spec.id)
                .unwrap_or_default()
                .to_string();
            MemberStatus {
                registered: registry.exists(&agent_id),
                session_active: session_active(&spec.id),
                member_id: spec.id.clone(),
                agent_id,
                name: spec.name.clone(),
                role: spec.role.clone(),
                description: spec.description.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::builtin_templates;
    use tempfile::tempdir;

    fn setup(tmp: &std::path::Path) -> (SwarmStore, AgentRegistry) {
        (SwarmStore::new(tmp), AgentRegistry::new(tmp, "RustyClaw"))
    }

    fn template(name: &str) -> SwarmConfig {
        let mut cfg = builtin_templates()[0].clone();
        cfg.name = name.to_string();
        cfg
    }

    #[test]
    fn create_materializes_registry_agents() {
        let tmp = tempdir().unwrap();
        let (store, registry) = setup(tmp.path());

        let record = create_swarm(&store, &registry, template("crew")).unwrap();
        assert_eq!(record.config.name, "crew");
        assert_eq!(record.members.len(), record.config.agents.len());

        // Every member is a real registered agent with provenance + prompt.
        for binding in &record.members {
            assert!(registry.exists(&binding.agent_id), "{}", binding.agent_id);
            let manifest = registry.manifest(&binding.agent_id).unwrap();
            assert_eq!(manifest.created_by.as_deref(), Some("swarm:crew"));
            let prompt = manifest.system_prompt.unwrap();
            assert!(prompt.contains("member of the 'crew' swarm"));
            assert!(prompt.contains("## Swarm roster"));
        }

        // The orchestrator's prompt lists its delegation targets by
        // registry agent id.
        let orch = registry.manifest("crew-orchestrator").unwrap();
        assert!(orch.system_prompt.unwrap().contains("crew-deep_research"));

        assert!(store.exists("crew"));
    }

    #[test]
    fn create_rejects_duplicates_and_rolls_back_on_conflict() {
        let tmp = tempdir().unwrap();
        let (store, registry) = setup(tmp.path());

        create_swarm(&store, &registry, template("crew")).unwrap();
        assert!(create_swarm(&store, &registry, template("crew")).is_err());

        // Plant an agent that collides with one member of a new swarm; the
        // create must fail without leaving any half-made members behind.
        registry
            .create(Some("team-docs"), "Occupied", None, None, None)
            .unwrap();
        let before: Vec<String> = registry.list().iter().map(|a| a.id.clone()).collect();
        assert!(create_swarm(&store, &registry, template("team")).is_err());
        let after: Vec<String> = registry.list().iter().map(|a| a.id.clone()).collect();
        assert_eq!(before, after, "conflicting create must not leak agents");
        assert!(!store.exists("team"));
    }

    #[test]
    fn send_routes_to_orchestrator_and_reuses_sessions() {
        let tmp = tempdir().unwrap();
        let (store, registry) = setup(tmp.path());
        create_swarm(&store, &registry, template("router")).unwrap();

        let first = send_to_member(&store, &registry, "router", None, "hello").unwrap();
        assert_eq!(first.member_id, "orchestrator");
        assert_eq!(first.agent_id, "router-orchestrator");
        assert!(!first.reused);

        // Second send reuses the same active session.
        let second = send_to_member(&store, &registry, "router", None, "again").unwrap();
        assert!(second.reused);
        assert_eq!(second.session_key, first.session_key);

        // Explicit member, by member id or registry agent id.
        let by_member = send_to_member(&store, &registry, "router", Some("docs"), "write").unwrap();
        assert_eq!(by_member.agent_id, "router-docs");
        let by_agent_id =
            send_to_member(&store, &registry, "router", Some("router-docs"), "more").unwrap();
        assert!(by_agent_id.reused);

        // Unknown member errors with the roster.
        let err = send_to_member(&store, &registry, "router", Some("ghost"), "x")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Available:"));

        // Stop completes the active sessions; a later send spawns fresh.
        let stopped = stop_swarm_sessions(&store, "router").unwrap();
        assert_eq!(stopped, 2);
        let after_stop = send_to_member(&store, &registry, "router", None, "anew").unwrap();
        assert!(!after_stop.reused);
        assert_ne!(after_stop.session_key, first.session_key);
    }

    #[test]
    fn delete_respects_provenance_and_keep_agents() {
        let tmp = tempdir().unwrap();
        let (store, registry) = setup(tmp.path());
        let record = create_swarm(&store, &registry, template("prov")).unwrap();

        // Simulate one member having been replaced by hand: foreign
        // provenance must survive deletion.
        registry.delete("prov-docs").unwrap();
        registry
            .create(Some("prov-docs"), "Hand-made", None, None, None)
            .unwrap();

        let report = delete_swarm(&store, &registry, "prov", false).unwrap();
        assert!(report.agents_kept.contains(&"prov-docs".to_string()));
        assert_eq!(
            report.agents_deleted.len(),
            record.members.len() - 1,
            "all swarm-created agents removed"
        );
        assert!(registry.exists("prov-docs"));
        assert!(!registry.exists("prov-orchestrator"));
        assert!(!store.exists("prov"));

        // keep_agents leaves everything in the registry.
        create_swarm(&store, &registry, template("keeper")).unwrap();
        let report = delete_swarm(&store, &registry, "keeper", true).unwrap();
        assert!(report.agents_deleted.is_empty());
        assert!(registry.exists("keeper-orchestrator"));
        assert!(!store.exists("keeper"));
    }

    #[test]
    fn member_statuses_reflect_registry_and_sessions() {
        let tmp = tempdir().unwrap();
        let (store, registry) = setup(tmp.path());
        create_swarm(&store, &registry, template("stat")).unwrap();
        send_to_member(&store, &registry, "stat", Some("docs"), "task").unwrap();

        let record = store.get("stat").unwrap();
        let statuses = member_statuses(&record, &registry);
        assert_eq!(statuses.len(), record.config.agents.len());
        let docs = statuses.iter().find(|s| s.member_id == "docs").unwrap();
        assert!(docs.registered && docs.session_active);
        let orch = statuses
            .iter()
            .find(|s| s.member_id == "orchestrator")
            .unwrap();
        assert!(orch.registered && !orch.session_active);

        // A deleted backing agent shows as unregistered and blocks sends.
        registry.delete("stat-docs").unwrap();
        let statuses = member_statuses(&record, &registry);
        let docs = statuses.iter().find(|s| s.member_id == "docs").unwrap();
        assert!(!docs.registered);
        assert!(send_to_member(&store, &registry, "stat", Some("docs"), "x").is_err());
    }
}
