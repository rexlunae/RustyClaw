//! Dispatch for `CommandAction`s produced by slash-command handling in the TUI run loop.

use rustyclaw_view::anyhow::Result;
use rustyclaw_view::tokio;
use std::sync::mpsc as sync_mpsc;

use rustyclaw_core::commands::CommandAction;
use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{GatewayClient, GatewayCommand};
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::skills::SkillManager;
use rustyclaw_view::PromptAttachment;

use super::GwEvent;
use rustyclaw_view::anyhow::Context;

/// Apply one `CommandAction`. Returns `Ok(true)` when the app should quit.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_command_action(
    action: CommandAction,
    client: &GatewayClient,
    gw_tx: &sync_mpsc::Sender<GwEvent>,
    prompt_attachments: &mut Vec<PromptAttachment>,
    config: &mut Config,
    secrets_manager: &mut SecretsManager,
    skill_manager: &SkillManager,
) -> Result<bool> {
    match action {
        CommandAction::Quit => return Ok(true),
        CommandAction::AttachPromptFile(path) => {
            if !prompt_attachments.iter().any(|item| item.path == path) {
                prompt_attachments.push(PromptAttachment::from_file_path(path.clone()));
            }
            crate::app::events::emit(
                gw_tx,
                GwEvent::PromptAttachmentsChanged {
                    attachments: prompt_attachments.clone(),
                },
            );
        }
        CommandAction::AttachPromptDirectory(path) => {
            if !prompt_attachments.iter().any(|item| item.path == path) {
                prompt_attachments.push(PromptAttachment::from_directory_path(path.clone()));
            }
            crate::app::events::emit(
                gw_tx,
                GwEvent::PromptAttachmentsChanged {
                    attachments: prompt_attachments.clone(),
                },
            );
        }
        CommandAction::ClearPromptAttachments => {
            prompt_attachments.clear();
            crate::app::events::emit(
                gw_tx,
                GwEvent::PromptAttachmentsChanged {
                    attachments: prompt_attachments.clone(),
                },
            );
        }
        CommandAction::ShowSecrets => {
            // Request secrets list from the gateway daemon
            // (secrets live in the gateway's vault, not locally).
            client
                .send(GatewayCommand::SecretsList)
                .await
                .context("sending SecretsList")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ShowSkills => {
            let skills_list: Vec<_> = skill_manager
                .get_skills()
                .iter()
                .map(|s| rustyclaw_view::SkillInfoData {
                    name: s.name.clone(),
                    description: s.description.clone().unwrap_or_default(),
                    enabled: s.enabled,
                })
                .collect();
            crate::app::events::emit(
                gw_tx,
                GwEvent::ShowSkills {
                    skills: skills_list,
                },
            );
        }
        CommandAction::ShowToolPermissions => {
            let tool_names = rustyclaw_core::tools::all_tool_names();
            let tools: Vec<_> = tool_names
                .iter()
                .map(|name| {
                    let perm = config
                        .tool_permissions
                        .get(*name)
                        .cloned()
                        .unwrap_or_default();
                    rustyclaw_view::ToolPermInfoData {
                        name: name.to_string(),
                        permission: perm.badge().to_string(),
                        summary: rustyclaw_core::tools::tool_summary(name).to_string(),
                    }
                })
                .collect();
            crate::app::events::emit(gw_tx, GwEvent::ShowToolPerms { tools });
        }
        CommandAction::ThreadNew(label) => {
            // Send thread create to gateway
            client
                .send(GatewayCommand::ThreadCreate {
                    label: Some(label),
                    project_id: None,
                })
                .await
                .context("sending ThreadCreate")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ThreadList => {
            // Focus sidebar to show threads
            crate::app::events::emit(
                gw_tx,
                GwEvent::Info("Press Tab to focus sidebar and navigate threads.".to_string()),
            );
        }
        CommandAction::ThreadClose(id) => {
            // Send thread close to gateway
            client
                .send(GatewayCommand::ThreadClose { thread_id: id })
                .await
                .context("sending ThreadClose")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ThreadRename(id, new_label) => {
            // Send thread rename to gateway
            client
                .send(GatewayCommand::ThreadRename {
                    thread_id: id,
                    new_label,
                })
                .await
                .context("sending ThreadRename")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ThreadBackground => {
            // Background the current foreground thread by switching
            // to thread_id 0 (sentinel: no foreground thread).
            client
                .send(GatewayCommand::ThreadSwitch { thread_id: 0 })
                .await
                .context("sending ThreadSwitch")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
            crate::app::events::emit(
                gw_tx,
                GwEvent::Info(
                    "Current thread backgrounded. Use /thread fg <id> or sidebar to switch."
                        .to_string(),
                ),
            );
        }
        CommandAction::ThreadForeground(id) => {
            // Foreground a thread by ID — reuse ThreadSwitch
            client
                .send(GatewayCommand::ThreadSwitch { thread_id: id })
                .await
                .context("sending ThreadSwitch")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::SetModel(model_name) => {
            // /model only changes the model, never the provider.
            // The model name is used exactly as entered — on
            // OpenRouter, IDs like "anthropic/claude-opus-4-20250514"
            // include a provider prefix that is part of the model ID,
            // not a directive to switch providers.  Use /provider to
            // change providers.
            let existing_provider = config
                .model
                .as_ref()
                .map(|m| m.provider.clone())
                .unwrap_or_else(|| "openrouter".to_string());

            // Update config — keep the current provider, only change model
            config.model = Some(rustyclaw_core::config::ModelProvider {
                provider: existing_provider,
                model: Some(model_name.clone()),
                base_url: config.model.as_ref().and_then(|m| m.base_url.clone()),
            });

            // Save config and tell the gateway to reload so the
            // new model takes effect immediately (no restart needed).
            if let Err(e) = config.save(None) {
                crate::app::events::emit(
                    gw_tx,
                    GwEvent::error(format!("Failed to save config: {}", e)),
                );
            } else {
                crate::app::events::emit(
                    gw_tx,
                    GwEvent::Info(format!("Model set to {}. Reloading gateway…", model_name)),
                );
                // Send Reload so the gateway picks up the new config
                client
                    .send(GatewayCommand::Reload)
                    .await
                    .context("sending Reload")
                    .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
            }
        }
        CommandAction::SetProvider(provider_name) => {
            // Update config with new provider, keep existing model
            let existing_model = config.model.as_ref().and_then(|m| m.model.clone());
            let base_url = rustyclaw_core::providers::base_url_override_for_switch(
                &provider_name,
                config.model.as_ref().map(|m| m.provider.as_str()),
                config.model.as_ref().and_then(|m| m.base_url.clone()),
            );
            config.model = Some(rustyclaw_core::config::ModelProvider {
                provider: provider_name.clone(),
                model: existing_model,
                base_url,
            });

            // Save config and tell the gateway to reload
            if let Err(e) = config.save(None) {
                crate::app::events::emit(
                    gw_tx,
                    GwEvent::error(format!("Failed to save config: {}", e)),
                );
            } else {
                crate::app::events::emit(
                    gw_tx,
                    GwEvent::Info(format!(
                        "Provider set to {}. Reloading gateway…",
                        provider_name
                    )),
                );
                client
                    .send(GatewayCommand::Reload)
                    .await
                    .context("sending Reload")
                    .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
            }
        }
        CommandAction::GatewayReload => {
            // Send Reload to the gateway
            client
                .send(GatewayCommand::Reload)
                .await
                .context("sending Reload")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::FetchModels => {
            // Spawn an async task to fetch the live model list
            // from the provider API and send results back via
            // the GwEvent channel.
            let provider_id = config
                .model
                .as_ref()
                .map(|m| m.provider.clone())
                .unwrap_or_default();
            let base_url = config.model.as_ref().and_then(|m| m.base_url.clone());
            // Read the API key: try the encrypted vault first
            // (where onboarding stores it), then fall back to
            // environment variables.
            let api_key = rustyclaw_core::providers::secret_key_for_provider(&provider_id)
                .and_then(|key_name| {
                    secrets_manager
                        .get_secret(key_name, true)
                        .ok()
                        .flatten()
                        .or_else(|| std::env::var(key_name).ok())
                });

            let gw_tx2 = gw_tx.clone();
            tokio::spawn(async move {
                match rustyclaw_core::providers::fetch_models_detailed(
                    &provider_id,
                    api_key.as_deref(),
                    base_url.as_deref(),
                )
                .await
                {
                    Ok(models) => {
                        let count = models.len();
                        let display =
                            rustyclaw_core::providers::display_name_for_provider(&provider_id);
                        let _ = gw_tx2
                            .send(GwEvent::Info(
                                format!("{} models from {}:", count, display,),
                            ));
                        // Show models in batches to avoid
                        // flooding the channel.
                        let lines: Vec<String> = models.iter().map(|m| m.display_line()).collect();
                        for chunk in lines.chunks(20) {
                            let _ = gw_tx2.send(GwEvent::Info(chunk.join("\n")));
                        }
                        let _ =
                            gw_tx2.send(GwEvent::Info("Tip: /model <id> to switch".to_string()));
                    }
                    Err(e) => {
                        let _ = gw_tx2.send(GwEvent::error_from_err(&e));
                    }
                }
            });
        }
        CommandAction::ShowProviderSelector => {
            // Build the provider list (built-in + custom) and send it to the UI
            let all = rustyclaw_core::providers::all_providers();
            let providers: Vec<String> = all
                .iter()
                .map(|p| {
                    if rustyclaw_core::providers::is_custom_provider(p.id) {
                        format!("{} (custom)", p.display)
                    } else {
                        p.display.to_string()
                    }
                })
                .collect();
            let ids: Vec<String> = all.iter().map(|p| p.id.to_string()).collect();
            let hints: Vec<String> = all
                .iter()
                .map(|p| match p.auth_method {
                    rustyclaw_core::providers::AuthMethod::ApiKey => "apikey".to_string(),
                    rustyclaw_core::providers::AuthMethod::DeviceFlow => "deviceflow".to_string(),
                    rustyclaw_core::providers::AuthMethod::None => "none".to_string(),
                    rustyclaw_core::providers::AuthMethod::OptionalApiKey => "apikey".to_string(),
                })
                .collect();
            crate::app::events::emit(
                gw_tx,
                GwEvent::ShowProviderSelector {
                    providers,
                    provider_ids: ids,
                    auth_hints: hints,
                },
            );
        }
        CommandAction::ShowEngines => {
            // Open the panel immediately (loading state) and request data.
            crate::app::events::emit(gw_tx, GwEvent::ShowEngines);
            client
                .send(GatewayCommand::EngineList)
                .await
                .context("sending EngineList")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::EngineAction(engine, action) => {
            client
                .send(GatewayCommand::EngineAction { engine, action })
                .await
                .context("sending EngineAction")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::EngineModelList(engine) => {
            // Open the panel so the model list has somewhere to land.
            crate::app::events::emit(gw_tx, GwEvent::ShowEngines);
            client
                .send(GatewayCommand::EngineList)
                .await
                .context("sending EngineList")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
            client
                .send(GatewayCommand::EngineModelList { engine })
                .await
                .context("sending EngineModelList")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::EngineModelPull(engine, model) => {
            crate::app::events::emit(gw_tx, GwEvent::ShowEngines);
            client
                .send(GatewayCommand::EngineList)
                .await
                .context("sending EngineList")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
            client
                .send(GatewayCommand::EngineModelPull {
                    engine,
                    model,
                    expected_size_bytes: None,
                })
                .await
                .context("sending EngineModelPull")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::EngineModelSearch(query) => {
            // Query the Hugging Face Hub directly — discovery is a public
            // API call, so it doesn't need to round-trip via the gateway.
            let gw_tx2 = gw_tx.clone();
            tokio::spawn(async move {
                match rustyclaw_core::engines::hub::search_models(&query, true, 20).await {
                    Ok(models) if models.is_empty() => {
                        let _ = gw_tx2.send(GwEvent::Info(format!(
                            "No GGUF models found for '{}'.",
                            query
                        )));
                    }
                    Ok(models) => {
                        let _ = gw_tx2.send(GwEvent::Info(format!(
                            "{} GGUF models for '{}' (most downloaded first):",
                            models.len(),
                            query
                        )));
                        let lines: Vec<String> = models.iter().map(|m| m.display_line()).collect();
                        for chunk in lines.chunks(20) {
                            let _ = gw_tx2.send(GwEvent::Info(chunk.join("\n")));
                        }
                        let _ = gw_tx2.send(GwEvent::Info(
                            "Tip: /engines files <repo> to pick a quantization, \
                             /engines pull <engine> <repo> to download"
                                .to_string(),
                        ));
                    }
                    Err(e) => {
                        let _ = gw_tx2.send(GwEvent::error(format!("{:#}", e)));
                    }
                }
            });
        }
        CommandAction::EngineModelFiles(repo) => {
            let gw_tx2 = gw_tx.clone();
            tokio::spawn(async move {
                match rustyclaw_core::engines::hub::list_gguf_files(&repo).await {
                    Ok(files) if files.is_empty() => {
                        let _ = gw_tx2.send(GwEvent::Info(format!(
                            "No GGUF files in {} — is it a GGUF repo?",
                            repo
                        )));
                    }
                    Ok(files) => {
                        let _ = gw_tx2.send(GwEvent::Info(format!(
                            "{} GGUF files in {}:",
                            files.len(),
                            repo
                        )));
                        let lines: Vec<String> = files.iter().map(|f| f.display_line()).collect();
                        for chunk in lines.chunks(20) {
                            let _ = gw_tx2.send(GwEvent::Info(chunk.join("\n")));
                        }
                        let _ = gw_tx2.send(GwEvent::Info(format!(
                            "Tip: /engines pull <engine> {} to download",
                            repo
                        )));
                    }
                    Err(e) => {
                        let _ = gw_tx2.send(GwEvent::error(format!("{:#}", e)));
                    }
                }
            });
        }
        CommandAction::EngineModelAction(engine, model, action) => {
            client
                .send(GatewayCommand::EngineModelAction {
                    engine,
                    model,
                    action,
                    context_length: None,
                    extra_args: Vec::new(),
                })
                .await
                .context("sending EngineModelAction")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ClearMessages => {
            crate::app::events::emit(gw_tx, GwEvent::ClearMessages);
        }
        CommandAction::GatewayInfo => {
            crate::app::events::emit(gw_tx, GwEvent::ShowGatewayStatus);
        }
        CommandAction::ShowDownloads => {
            crate::app::events::emit(gw_tx, GwEvent::ShowDownloads);
            client
                .send(GatewayCommand::DownloadsRequest)
                .await
                .context("sending DownloadsRequest")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::DownloadCancel(id) => {
            // The panel is opened too: a cancel with nothing on screen gives
            // the user no way to see whether it took.
            crate::app::events::emit(gw_tx, GwEvent::ShowDownloads);
            client
                .send(GatewayCommand::DownloadCancel { id })
                .await
                .context("sending DownloadCancel")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::DownloadsClearFinished => {
            crate::app::events::emit(gw_tx, GwEvent::ShowDownloads);
            client
                .send(GatewayCommand::DownloadsClearFinished)
                .await
                .context("sending DownloadsClearFinished")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ShowCron => {
            crate::app::events::emit(gw_tx, GwEvent::ShowCron);
            client
                .send(GatewayCommand::CronList)
                .await
                .context("sending CronList")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::CronAdd(name, expr, payload) => {
            crate::app::events::emit(gw_tx, GwEvent::ShowCron);
            client
                .send(GatewayCommand::CronUpsert {
                    id: None,
                    name,
                    expr,
                    payload,
                    paused: false,
                })
                .await
                .context("sending CronUpsert")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::CronAction(id, action) => {
            crate::app::events::emit(gw_tx, GwEvent::ShowCron);
            client
                .send(GatewayCommand::CronAction { id, action })
                .await
                .context("sending CronAction")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ShowMemory(query) => {
            crate::app::events::emit(
                gw_tx,
                GwEvent::ShowMemory {
                    query: query.clone(),
                },
            );
            client
                .send(GatewayCommand::MemoryList { query, limit: None })
                .await
                .context("sending MemoryList")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::MemoryAdd(category, content) => {
            crate::app::events::emit(gw_tx, GwEvent::ShowMemory { query: None });
            client
                .send(GatewayCommand::MemoryUpsert {
                    id: None,
                    content,
                    category,
                })
                .await
                .context("sending MemoryUpsert")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::MemoryDelete(id) => {
            crate::app::events::emit(gw_tx, GwEvent::ShowMemory { query: None });
            client
                .send(GatewayCommand::MemoryDelete { id })
                .await
                .context("sending MemoryDelete")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::HistorySearch(query) => {
            crate::app::events::emit(
                gw_tx,
                GwEvent::ShowMemory {
                    query: Some(query.clone()),
                },
            );
            // Populate both halves of the memory panel.
            client
                .send(GatewayCommand::MemoryList {
                    query: Some(query.clone()),
                    limit: None,
                })
                .await
                .context("sending MemoryList")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
            client
                .send(GatewayCommand::HistorySearch { query, limit: None })
                .await
                .context("sending HistorySearch")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ShowMcp => {
            crate::app::events::emit(gw_tx, GwEvent::ShowMcp);
            client
                .send(GatewayCommand::McpList)
                .await
                .context("sending McpList")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::McpConnect(name, command) => {
            crate::app::events::emit(gw_tx, GwEvent::ShowMcp);
            client
                .send(GatewayCommand::McpConnect {
                    name,
                    command,
                    url: None,
                    env: Vec::new(),
                })
                .await
                .context("sending McpConnect")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::McpDisconnect(name) => {
            crate::app::events::emit(gw_tx, GwEvent::ShowMcp);
            client
                .send(GatewayCommand::McpDisconnect { name })
                .await
                .context("sending McpDisconnect")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ShowMessengers => {
            crate::app::events::emit(gw_tx, GwEvent::ShowMessengers);
            client
                .send(GatewayCommand::MessengerConfig)
                .await
                .context("sending MessengerConfig")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ShowChannels => {
            crate::app::events::emit(gw_tx, GwEvent::ShowChannels);
            client
                .send(GatewayCommand::ChannelStatus)
                .await
                .context("sending ChannelStatus")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ChannelPair(channel, action) => {
            crate::app::events::emit(gw_tx, GwEvent::ShowChannels);
            client
                .send(GatewayCommand::ChannelPair { channel, action })
                .await
                .context("sending ChannelPair")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ShowAnalytics(period) => {
            crate::app::events::emit(gw_tx, GwEvent::ShowAnalytics);
            client
                .send(GatewayCommand::UsageStats { period })
                .await
                .context("sending UsageStats")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ShowLogs(source, tail) => {
            crate::app::events::emit(
                gw_tx,
                GwEvent::ShowLogs {
                    source: source.clone(),
                },
            );
            client
                .send(GatewayCommand::Logs { source, tail })
                .await
                .context("sending Logs")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::ShowAgents => {
            // Open the selector immediately; the list arrives via AgentsUpdate.
            crate::app::events::emit(gw_tx, GwEvent::ShowAgentSelector);
            client
                .send(GatewayCommand::AgentList)
                .await
                .context("sending AgentList")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::AgentSwitch(agent_id) => {
            client
                .send(GatewayCommand::AgentSwitch { agent_id })
                .await
                .context("sending AgentSwitch")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::AgentNew(name) => {
            client
                .send(GatewayCommand::AgentCreate {
                    name,
                    agent_id: None,
                    description: None,
                })
                .await
                .context("sending AgentCreate")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        CommandAction::AgentDelete(agent_id) => {
            client
                .send(GatewayCommand::AgentDelete { agent_id })
                .await
                .context("sending AgentDelete")
                .unwrap_or_else(|e| crate::app::events::report(gw_tx, e));
        }
        _ => {}
    }
    Ok(false)
}
