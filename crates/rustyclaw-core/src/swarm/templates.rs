//! Built-in swarm templates.
//!
//! The default template mirrors the OpenSwarm agent roster: an orchestrator
//! plus seven specialists covering research, data analysis, slides, docs,
//! image generation, video generation, and general virtual-assistant tasks.

use super::config::{
    AgentRole, CommunicationFlow, FlowKind, SwarmAgent, SwarmConfig,
};

/// Shared instructions injected into every agent in the default swarm.
const DEFAULT_SHARED_INSTRUCTIONS: &str = "\
You are part of a multi-agent swarm managed by RustyClaw.  These instructions \
apply to every agent in the swarm.\n\n\
## Runtime Environment\n\
- You are running locally on the user's machine via the RustyClaw runtime.\n\
- Communicate directly with the user through the chat interface.\n\n\
## File Delivery\n\
- When you generate files, include the full file path in your response.\n\
- Use the `write_file` tool to persist deliverables.\n\n\
## Agent-to-Agent Communication\n\
- The orchestrator routes tasks; specialists execute them.\n\
- Use `sessions_send` to communicate with other agents when needed.\n\
- Return results to the orchestrator promptly so it can merge outputs.";

/// Build the default OpenSwarm-style template.
fn default_openswarm_template() -> SwarmConfig {
    let agents = vec![
        SwarmAgent {
            id: "orchestrator".into(),
            name: "Orchestrator".into(),
            role: AgentRole::Orchestrator,
            instructions: "You are the orchestrator — the main entry-point for the swarm.\n\
                Your only job is to turn user goals into the right multi-agent \
                execution strategy and route work to specialists.  You never \
                execute tasks yourself.\n\n\
                ## Routing Guide\n\
                - Virtual Assistant: admin workflows, external systems, scheduling.\n\
                - Deep Research: evidence-based research with citations.\n\
                - Data Analyst: data analysis, KPIs, charts.\n\
                - Slides Agent: presentation creation and export.\n\
                - Docs Agent: document creation (PDF, Markdown, DOCX).\n\
                - Image Agent: image generation and editing.\n\
                - Video Agent: video generation and editing.\n\n\
                ## Communication Patterns\n\
                - Use SendMessage (parallel) when subtasks are independent.\n\
                - Use Handoff when a single specialist should take over."
                .into(),
            description: "Routes tasks to the right specialist(s); never executes directly."
                .into(),
            tools: vec![
                "sessions_spawn".into(),
                "sessions_send".into(),
                "sessions_list".into(),
                "sessions_history".into(),
                "session_status".into(),
                "swarm_status".into(),
            ],
            conversation_starters: vec![
                "What can this swarm do?".into(),
                "Build a launch package: research, slides, docs, and creative assets.".into(),
                "Analyze my data and turn insights into an executive deck.".into(),
            ],
        },
        SwarmAgent {
            id: "virtual_assistant".into(),
            name: "Virtual Assistant".into(),
            role: AgentRole::VirtualAssistant,
            instructions: "You are a general-purpose virtual assistant.\n\
                Handle everyday tasks: writing, scheduling, messaging, and task management.\n\
                You have access to shell commands, web search, and file tools.\n\
                For external integrations, use MCP servers or available skills."
                .into(),
            description:
                "Handles admin workflows, external systems, messaging, scheduling."
                    .into(),
            tools: vec![
                "execute_command".into(),
                "web_search".into(),
                "web_fetch".into(),
                "read_file".into(),
                "write_file".into(),
                "edit_file".into(),
                "mcp_list".into(),
                "mcp_connect".into(),
                "skill_list".into(),
                "cron".into(),
                "message".into(),
            ],
            conversation_starters: vec![
                "Send a summary of my unread emails.".into(),
                "Schedule a meeting for next Monday.".into(),
                "What external systems do I have connected?".into(),
            ],
        },
        SwarmAgent {
            id: "deep_research".into(),
            name: "Deep Research Agent".into(),
            role: AgentRole::DeepResearch,
            instructions: "You are a deep research specialist.\n\
                Conduct comprehensive, evidence-based research on any topic.\n\
                Always cite sources and present balanced analysis.\n\
                Use web search extensively and cross-reference multiple sources.\n\
                Return structured reports with sections and citations."
                .into(),
            description:
                "Evidence-based web/academic research with citations and analysis."
                    .into(),
            tools: vec![
                "web_search".into(),
                "web_fetch".into(),
                "read_file".into(),
                "write_file".into(),
                "memory_search".into(),
                "save_memory".into(),
            ],
            conversation_starters: vec![
                "Research the latest trends in renewable energy.".into(),
                "Comprehensive analysis of the AI agent market.".into(),
                "Compare the top 5 project management tools.".into(),
            ],
        },
        SwarmAgent {
            id: "data_analyst".into(),
            name: "Data Analyst".into(),
            role: AgentRole::DataAnalyst,
            instructions: "You are an advanced data analytics agent.\n\
                Analyze structured data, build charts, run statistical models.\n\
                Use shell commands to run Python/R scripts for analysis.\n\
                Generate visualisations and save them as image files.\n\
                Present actionable insights with clear KPIs."
                .into(),
            description:
                "Data analysis, KPIs, charts, and statistical modelling."
                    .into(),
            tools: vec![
                "execute_command".into(),
                "read_file".into(),
                "write_file".into(),
                "web_search".into(),
                "web_fetch".into(),
                "find_files".into(),
            ],
            conversation_starters: vec![
                "Analyze this CSV and show me key trends.".into(),
                "Create a dashboard with charts from my sales data.".into(),
                "Find hidden patterns in this dataset.".into(),
            ],
        },
        SwarmAgent {
            id: "slides".into(),
            name: "Slides Agent".into(),
            role: AgentRole::Slides,
            instructions: "You are a presentation specialist.\n\
                Create polished slide decks in HTML format and export to PPTX.\n\
                Use clear visual hierarchy, concise bullet points, and \
                appropriate imagery.  Save outputs as files and report paths.\n\
                For images, use the image tool or web_fetch to download them."
                .into(),
            description:
                "Slide deck creation, editing, and export (HTML → PPTX)."
                    .into(),
            tools: vec![
                "execute_command".into(),
                "read_file".into(),
                "write_file".into(),
                "edit_file".into(),
                "web_search".into(),
                "web_fetch".into(),
                "image".into(),
            ],
            conversation_starters: vec![
                "Create a presentation about AI in the workplace.".into(),
                "Build a pitch deck for my startup.".into(),
                "Turn this document into a slide deck.".into(),
            ],
        },
        SwarmAgent {
            id: "docs".into(),
            name: "Docs Agent".into(),
            role: AgentRole::Docs,
            instructions: "You are a professional document engineer.\n\
                Create, edit, and convert documents to multiple formats \
                (PDF, Markdown, TXT, DOCX).  Use clear structure with \
                headings, bullet points, and tables.  For PDF generation, \
                use command-line tools like pandoc or wkhtmltopdf via \
                execute_command.  Save outputs and report file paths."
                .into(),
            description:
                "Document creation and conversion (PDF, Markdown, DOCX)."
                    .into(),
            tools: vec![
                "execute_command".into(),
                "read_file".into(),
                "write_file".into(),
                "edit_file".into(),
                "web_search".into(),
            ],
            conversation_starters: vec![
                "Draft a client status report and export as PDF.".into(),
                "Create a one-page proposal as DOCX.".into(),
                "Write an onboarding SOP in Markdown.".into(),
            ],
        },
        SwarmAgent {
            id: "image_gen".into(),
            name: "Image Agent".into(),
            role: AgentRole::ImageGeneration,
            instructions: "You are an image generation and editing specialist.\n\
                Generate images using available AI providers and tools.\n\
                Edit existing images, compose layouts, and create visual assets.\n\
                Save all generated images as files and report their paths."
                .into(),
            description:
                "AI image generation, editing, and composition."
                    .into(),
            tools: vec![
                "image".into(),
                "execute_command".into(),
                "read_file".into(),
                "write_file".into(),
                "web_fetch".into(),
            ],
            conversation_starters: vec![
                "Generate a product hero image for my landing page.".into(),
                "Edit this photo to match a cinematic style.".into(),
                "Create two image variants with different styles.".into(),
            ],
        },
        SwarmAgent {
            id: "video_gen".into(),
            name: "Video Agent".into(),
            role: AgentRole::VideoGeneration,
            instructions: "You are a video generation and editing specialist.\n\
                Produce videos using available AI providers and CLI tools \
                (ffmpeg, etc.).  Edit clips, add captions, combine footage.\n\
                Save all generated videos as files and report their paths."
                .into(),
            description:
                "AI video generation, editing, and assembly."
                    .into(),
            tools: vec![
                "execute_command".into(),
                "read_file".into(),
                "write_file".into(),
                "web_search".into(),
                "web_fetch".into(),
            ],
            conversation_starters: vec![
                "Generate a short promo video for my product launch.".into(),
                "Create an animated explainer about AI.".into(),
                "Edit this video clip and add captions.".into(),
            ],
        },
    ];

    // Build communication flows: orchestrator → each specialist (SendMessage)
    // and bidirectional handoffs between all agents.
    let agent_ids: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
    let mut flows = Vec::new();

    for id in &agent_ids {
        if id == "orchestrator" {
            continue;
        }
        // Orchestrator can send messages (parallel delegation) to all specialists.
        flows.push(CommunicationFlow {
            from: "orchestrator".into(),
            to: id.clone(),
            kind: FlowKind::SendMessage,
        });
    }

    // All agents can hand off to any other agent.
    for a in &agent_ids {
        for b in &agent_ids {
            if a != b {
                flows.push(CommunicationFlow {
                    from: a.clone(),
                    to: b.clone(),
                    kind: FlowKind::Handoff,
                });
            }
        }
    }

    SwarmConfig {
        name: "openswarm".into(),
        description: "Default multi-agent swarm — orchestrator + 7 specialists \
            covering research, data analysis, slides, docs, images, video, and \
            general assistant tasks.  Inspired by VRSEN/OpenSwarm."
            .into(),
        shared_instructions: DEFAULT_SHARED_INSTRUCTIONS.into(),
        agents,
        flows,
    }
}

/// Return all built-in swarm templates.
pub fn builtin_templates() -> Vec<SwarmConfig> {
    vec![default_openswarm_template()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_has_8_agents() {
        let templates = builtin_templates();
        assert_eq!(templates.len(), 1);
        let t = &templates[0];
        assert_eq!(t.name, "openswarm");
        assert_eq!(t.agents.len(), 8);
    }

    #[test]
    fn orchestrator_is_first_agent() {
        let t = &builtin_templates()[0];
        assert_eq!(t.agents[0].role, AgentRole::Orchestrator);
    }

    #[test]
    fn flows_include_send_message_and_handoff() {
        let t = &builtin_templates()[0];
        let has_send = t.flows.iter().any(|f| f.kind == FlowKind::SendMessage);
        let has_handoff = t.flows.iter().any(|f| f.kind == FlowKind::Handoff);
        assert!(has_send);
        assert!(has_handoff);
    }

    #[test]
    fn orchestrator_has_sendmessage_to_all_specialists() {
        let t = &builtin_templates()[0];
        let specialist_ids: Vec<&str> = t
            .agents
            .iter()
            .filter(|a| a.role != AgentRole::Orchestrator)
            .map(|a| a.id.as_str())
            .collect();

        for sid in specialist_ids {
            let has_flow = t.flows.iter().any(|f| {
                f.from == "orchestrator" && f.to == sid && f.kind == FlowKind::SendMessage
            });
            assert!(has_flow, "Missing SendMessage flow to {sid}");
        }
    }
}
