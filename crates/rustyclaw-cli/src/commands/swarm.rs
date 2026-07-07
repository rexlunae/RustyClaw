//! `swarm` command: create and drive multi-agent swarms.

use anyhow::Result;
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub(crate) enum SwarmCommands {
    /// Create a new swarm from a template
    Create {
        /// Template name (default: 'swarm')
        #[arg(value_name = "TEMPLATE", default_value = "swarm")]
        template: String,
    },
    /// List all swarms
    List,
    /// Show detailed status of a swarm
    Status {
        /// Swarm name
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Send a message/task to a swarm agent
    Send {
        /// Swarm name
        #[arg(value_name = "SWARM")]
        swarm: String,
        /// Message to send
        #[arg(value_name = "MESSAGE", trailing_var_arg = true)]
        message: Vec<String>,
        /// Target agent ID (default: orchestrator)
        #[arg(long, short, value_name = "AGENT")]
        agent: Option<String>,
    },
    /// Stop a running swarm
    Stop {
        /// Swarm name
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// List available swarm templates
    Templates,
}

/// Run a `swarm` subcommand.
pub(crate) fn run(sub: SwarmCommands) -> Result<()> {
    use rustyclaw_core::swarm::{builtin_templates, swarm_manager};
    use rustyclaw_core::theme as t;

    match sub {
        SwarmCommands::Create { template } => {
            let templates = builtin_templates();
            let cfg = templates
                .into_iter()
                .find(|t| t.name == template)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unknown template '{}'. Use `rustyclaw swarm templates` to list.",
                        template
                    )
                })?;
            let name = cfg.name.clone();
            let agent_count = cfg.agents.len();
            let mgr = swarm_manager();
            let mut m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
            m.create(cfg)?;
            m.start(&name)?;
            println!(
                "{}",
                t::icon_ok(&format!(
                    "Swarm '{}' created and started with {} agents",
                    name, agent_count
                ))
            );

            let inst = m.get(&name).expect("just created");
            for agent in &inst.config.agents {
                println!("  • {} ({})", t::accent_bright(&agent.name), agent.role);
            }
        }
        SwarmCommands::List => {
            let mgr = swarm_manager();
            let m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
            let swarms = m.list();
            if swarms.is_empty() {
                println!(
                    "{}",
                    t::muted("No swarms defined. Use `rustyclaw swarm create` to create one.")
                );
            } else {
                for inst in swarms {
                    let status = match inst.status {
                        rustyclaw_core::swarm::SwarmStatus::Running => t::icon_ok("Running"),
                        rustyclaw_core::swarm::SwarmStatus::Idle => t::info("Idle"),
                        rustyclaw_core::swarm::SwarmStatus::Paused => t::info("Paused"),
                        rustyclaw_core::swarm::SwarmStatus::Stopped => t::muted("Stopped"),
                        rustyclaw_core::swarm::SwarmStatus::Error => t::icon_fail("Error"),
                    };
                    println!(
                        "  {} {} — {} agents, {} tasks",
                        status,
                        t::accent_bright(&inst.config.name),
                        inst.config.agents.len(),
                        inst.tasks_routed,
                    );
                }
            }
        }
        SwarmCommands::Status { name } => {
            let mgr = swarm_manager();
            let m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
            let inst = m
                .get(&name)
                .ok_or_else(|| anyhow::anyhow!("Swarm '{}' not found", name))?;
            println!(
                "{} — {} ({}s uptime, {} tasks)",
                t::accent_bright(&inst.config.name),
                inst.status,
                inst.runtime_secs(),
                inst.tasks_routed,
            );
            println!();
            println!("{}", t::info("Agents:"));
            for agent in &inst.config.agents {
                let session = inst
                    .agent_sessions
                    .get(&agent.id)
                    .map(|s| format!(" [{}]", s))
                    .unwrap_or_default();
                println!(
                    "  • {} ({}){}",
                    t::accent_bright(&agent.name),
                    agent.role,
                    session
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
            let mgr = swarm_manager();
            let target = agent.as_deref().unwrap_or("orchestrator");

            // Phase 1: validate swarm/agent and extract info.
            let (agent_name, agent_instructions, existing_session) = {
                let mut m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
                let inst = m
                    .get_mut(&swarm)
                    .ok_or_else(|| anyhow::anyhow!("Swarm '{}' not found", swarm))?;
                if inst.status != rustyclaw_core::swarm::SwarmStatus::Running {
                    anyhow::bail!("Swarm '{}' is not running", swarm);
                }
                let a = inst
                    .config
                    .agents
                    .iter()
                    .find(|a| a.id == target)
                    .ok_or_else(|| {
                        let ids: Vec<&str> =
                            inst.config.agents.iter().map(|a| a.id.as_str()).collect();
                        anyhow::anyhow!(
                            "Agent '{}' not found in swarm '{}'. Available: {}",
                            target,
                            swarm,
                            ids.join(", ")
                        )
                    })?;
                let name = a.name.clone();
                let instructions = a.instructions.clone();
                let existing = inst.agent_sessions.get(target).cloned();
                inst.record_task();
                (name, instructions, existing)
            };

            // Phase 2: route via session manager (no swarm lock held).
            let session_mgr = rustyclaw_core::sessions::session_manager();
            let mut sess_mgr = session_mgr
                .lock()
                .map_err(|_| anyhow::anyhow!("Session manager lock error"))?;

            let session_key = if let Some(existing) = existing_session {
                sess_mgr.send_message(&existing, &msg)?;
                existing
            } else {
                let label = format!("swarm:{}:{}", swarm, target);
                let task = format!(
                    "[Swarm: {} | Agent: {}]\n\n{}\n\nSystem Instructions:\n{}",
                    swarm, agent_name, msg, agent_instructions
                );
                let key = sess_mgr.spawn_subagent(target, &task, Some(label), None);
                drop(sess_mgr);

                // Phase 3: store session key back.
                let mut m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
                if let Some(inst) = m.get_mut(&swarm) {
                    inst.agent_sessions.insert(target.to_string(), key.clone());
                }
                key
            };

            println!(
                "{}",
                t::icon_ok(&format!(
                    "Task routed to {} ({}) in swarm '{}' — session: {}",
                    agent_name, target, swarm, session_key
                ))
            );
            println!("  Message: {}", t::muted(&msg));
        }
        SwarmCommands::Stop { name } => {
            let mgr = swarm_manager();
            let mut m = mgr.lock().map_err(|_| anyhow::anyhow!("Lock error"))?;
            m.stop(&name)?;
            println!("{}", t::icon_ok(&format!("Swarm '{}' stopped", name)));
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
