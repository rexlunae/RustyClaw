//! `swarm` command: create and drive multi-agent swarms.
//!
//! Swarms are persistent groups of registered agents recorded under
//! `<settings_dir>/swarms/`, so this command shares state with the gateway
//! and model tools instead of operating on process-local memory.

use anyhow::Result;
use clap::Subcommand;
use rustyclaw_core::agents::AgentRegistry;
use rustyclaw_core::config::Config;
use rustyclaw_core::swarm::SwarmStore;

#[derive(Debug, Subcommand)]
pub(crate) enum SwarmCommands {
    /// Create a new swarm from a template
    Create {
        /// Template name (default: 'swarm')
        #[arg(value_name = "TEMPLATE", default_value = "swarm")]
        template: String,
        /// Name for the new swarm (defaults to the template's name)
        #[arg(long, short, value_name = "NAME")]
        name: Option<String>,
    },
    /// List all swarms
    List,
    /// Show detailed status of a swarm
    Status {
        /// Swarm name
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Send a message/task to a swarm member
    Send {
        /// Swarm name
        #[arg(value_name = "SWARM")]
        swarm: String,
        /// Message to send
        #[arg(value_name = "MESSAGE", trailing_var_arg = true)]
        message: Vec<String>,
        /// Target member ID (default: the orchestrator)
        #[arg(long, short, value_name = "AGENT")]
        agent: Option<String>,
    },
    /// Complete a swarm's active sessions (agents remain)
    Stop {
        /// Swarm name
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Delete a swarm and the agents it created
    Delete {
        /// Swarm name
        #[arg(value_name = "NAME")]
        name: String,
        /// Keep the member agents in the registry
        #[arg(long)]
        keep_agents: bool,
    },
    /// List available swarm templates
    Templates,
}

/// The store and registry rooted at this installation's settings dir.
fn swarm_context(config: &Config) -> (SwarmStore, AgentRegistry) {
    (
        SwarmStore::new(&config.settings_dir),
        AgentRegistry::new(&config.settings_dir, &config.agent_name),
    )
}

/// Run a `swarm` subcommand.
pub(crate) fn run(sub: SwarmCommands, config: &Config) -> Result<()> {
    use rustyclaw_core::swarm::{
        builtin_templates, create_swarm, delete_swarm, member_statuses, send_to_member,
        stop_swarm_sessions,
    };
    use rustyclaw_core::theme as t;

    let (store, registry) = swarm_context(config);

    match sub {
        SwarmCommands::Create { template, name } => {
            let mut cfg = builtin_templates()
                .into_iter()
                .find(|t| t.name == template)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unknown template '{}'. Use `rustyclaw swarm templates` to list.",
                        template
                    )
                })?;
            if let Some(name) = name {
                cfg.name = name;
            }
            let record = create_swarm(&store, &registry, cfg)?;
            println!(
                "{}",
                t::icon_ok(&format!(
                    "Swarm '{}' created — {} members registered as agents",
                    record.config.name,
                    record.members.len()
                ))
            );
            for member in &record.config.agents {
                println!(
                    "  • {} ({}) → {}",
                    t::accent_bright(&member.name),
                    member.role,
                    record.agent_id_for(&member.id).unwrap_or("?"),
                );
            }
        }
        SwarmCommands::List => {
            let swarms = store.list();
            if swarms.is_empty() {
                println!(
                    "{}",
                    t::muted("No swarms defined. Use `rustyclaw swarm create` to create one.")
                );
            } else {
                for record in swarms {
                    let statuses = member_statuses(&record, &registry);
                    let registered = statuses.iter().filter(|s| s.registered).count();
                    let active = statuses.iter().filter(|s| s.session_active).count();
                    let health = if registered == statuses.len() {
                        t::icon_ok("ready")
                    } else {
                        t::icon_fail("degraded")
                    };
                    println!(
                        "  {} {} — {} agents ({} registered, {} active sessions)",
                        health,
                        t::accent_bright(&record.config.name),
                        statuses.len(),
                        registered,
                        active,
                    );
                }
            }
        }
        SwarmCommands::Status { name } => {
            let record = store
                .get(&name)
                .ok_or_else(|| anyhow::anyhow!("Swarm '{}' not found", name))?;
            let statuses = member_statuses(&record, &registry);
            let registered = statuses.iter().filter(|s| s.registered).count();
            let health = if registered == statuses.len() {
                "ready".to_string()
            } else {
                format!(
                    "degraded ({}/{} agents present)",
                    registered,
                    statuses.len()
                )
            };
            println!(
                "{} — {} ({}s old)",
                t::accent_bright(&record.config.name),
                health,
                record.age_secs(),
            );
            println!();
            println!("{}", t::info("Members:"));
            for s in &statuses {
                println!(
                    "  • {} ({}) → {}{}{}",
                    t::accent_bright(&s.name),
                    s.role,
                    s.agent_id,
                    if s.registered { "" } else { " [missing]" },
                    if s.session_active {
                        " [session active]"
                    } else {
                        ""
                    },
                );
            }
        }
        SwarmCommands::Send {
            swarm,
            message,
            agent,
        } => {
            let msg = message.join(" ");
            if msg.trim().is_empty() {
                anyhow::bail!("Message cannot be empty");
            }
            let route = send_to_member(&store, &registry, &swarm, agent.as_deref(), &msg)?;
            println!(
                "{}",
                t::icon_ok(&format!(
                    "Task routed to {} ({}) in swarm '{}' — {} session {}",
                    route.member_name,
                    route.agent_id,
                    swarm,
                    if route.reused { "existing" } else { "new" },
                    route.session_key,
                ))
            );
            println!("  Message: {}", t::muted(&msg));
        }
        SwarmCommands::Stop { name } => {
            let completed = stop_swarm_sessions(&store, &name)?;
            println!(
                "{}",
                t::icon_ok(&format!(
                    "Swarm '{}': {} active session(s) completed (agents remain)",
                    name, completed
                ))
            );
        }
        SwarmCommands::Delete { name, keep_agents } => {
            let report = delete_swarm(&store, &registry, &name, keep_agents)?;
            println!(
                "{}",
                t::icon_ok(&format!(
                    "Swarm '{}' deleted — {} agent(s) removed, {} kept",
                    name,
                    report.agents_deleted.len(),
                    report.agents_kept.len(),
                ))
            );
        }
        SwarmCommands::Templates => {
            let templates = builtin_templates();
            println!("{}", t::accent_bright("Available swarm templates:"));
            println!();
            for t_cfg in &templates {
                println!(
                    "  {} — {} agents",
                    t::accent_bright(&t_cfg.name),
                    t_cfg.agents.len()
                );
                println!("    {}", t::muted(&t_cfg.description));
                for agent in &t_cfg.agents {
                    println!("      • {} ({})", agent.name, agent.role);
                }
                println!();
            }
        }
    }
    Ok(())
}
