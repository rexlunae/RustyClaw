//! Dispatch for `CommandAction`s produced by slash-command handling in the TUI run loop.

use anyhow::Result;
use std::sync::mpsc as sync_mpsc;

use rustyclaw_core::commands::CommandAction;
use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{GatewayClient, GatewayCommand};
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::skills::SkillManager;
use rustyclaw_view::PromptAttachment;

use super::GwEvent;

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
            let _ = gw_tx.send(GwEvent::PromptAttachmentsChanged {
                attachments: prompt_attachments.clone(),
            });
        }
        CommandAction::AttachPromptDirectory(path) => {
            if !prompt_attachments.iter().any(|item| item.path == path) {
                prompt_attachments.push(PromptAttachment::from_directory_path(path.clone()));
            }
            let _ = gw_tx.send(GwEvent::PromptAttachmentsChanged {
                attachments: prompt_attachments.clone(),
            });
        }
        CommandAction::ClearPromptAttachments => {
            prompt_attachments.clear();
            let _ = gw_tx.send(GwEvent::PromptAttachmentsChanged {
                attachments: prompt_attachments.clone(),
            });
        }
        CommandAction::ShowSecrets => {
            // Request secrets list from the gateway daemon
            // (secrets live in the gateway's vault, not locally).
            let _ = client.send(GatewayCommand::SecretsList).await;
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
            let _ = gw_tx.send(GwEvent::ShowSkills {
                skills: skills_list,
            });
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
            let _ = gw_tx.send(GwEvent::ShowToolPerms { tools });
        }
        CommandAction::ThreadNew(label) => {
            // Send thread create to gateway
            let _ = client
                .send(GatewayCommand::ThreadCreate { label: Some(label) })
                .await;
        }
        CommandAction::ThreadList => {
            // Focus sidebar to show threads
            let _ = gw_tx.send(GwEvent::Info(
                "Press Tab to focus sidebar and navigate threads.".to_string(),
            ));
        }
        CommandAction::ThreadClose(id) => {
            // Send thread close to gateway
            let _ = client
                .send(GatewayCommand::ThreadClose { thread_id: id })
                .await;
        }
        CommandAction::ThreadRename(id, new_label) => {
            // Send thread rename to gateway
            let _ = client
                .send(GatewayCommand::ThreadRename {
                    thread_id: id,
                    new_label,
                })
                .await;
        }
        CommandAction::ThreadBackground => {
            // Background the current foreground thread by switching
            // to thread_id 0 (sentinel: no foreground thread).
            let _ = client
                .send(GatewayCommand::ThreadSwitch { thread_id: 0 })
                .await;
            let _ = gw_tx.send(GwEvent::Info(
                "Current thread backgrounded. Use /thread fg <id> or sidebar to switch."
                    .to_string(),
            ));
        }
        CommandAction::ThreadForeground(id) => {
            // Foreground a thread by ID — reuse ThreadSwitch
            let _ = client
                .send(GatewayCommand::ThreadSwitch { thread_id: id })
                .await;
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
                let _ = gw_tx.send(GwEvent::error(format!("Failed to save config: {}", e)));
            } else {
                let _ = gw_tx.send(GwEvent::Info(format!(
                    "Model set to {}. Reloading gateway…",
                    model_name
                )));
                // Send Reload so the gateway picks up the new config
                let _ = client.send(GatewayCommand::Reload).await;
            }
        }
        CommandAction::SetProvider(provider_name) => {
            // Update config with new provider, keep existing model
            let existing_model = config.model.as_ref().and_then(|m| m.model.clone());
            config.model = Some(rustyclaw_core::config::ModelProvider {
                provider: provider_name.clone(),
                model: existing_model,
                base_url: config.model.as_ref().and_then(|m| m.base_url.clone()),
            });

            // Save config and tell the gateway to reload
            if let Err(e) = config.save(None) {
                let _ = gw_tx.send(GwEvent::error(format!("Failed to save config: {}", e)));
            } else {
                let _ = gw_tx.send(GwEvent::Info(format!(
                    "Provider set to {}. Reloading gateway…",
                    provider_name
                )));
                let _ = client.send(GatewayCommand::Reload).await;
            }
        }
        CommandAction::GatewayReload => {
            // Send Reload to the gateway
            let _ = client.send(GatewayCommand::Reload).await;
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
            // Build the provider list and send it to the UI
            let providers: Vec<String> = rustyclaw_core::providers::PROVIDERS
                .iter()
                .map(|p| p.display.to_string())
                .collect();
            let ids: Vec<String> = rustyclaw_core::providers::PROVIDERS
                .iter()
                .map(|p| p.id.to_string())
                .collect();
            let hints: Vec<String> = rustyclaw_core::providers::PROVIDERS
                .iter()
                .map(|p| match p.auth_method {
                    rustyclaw_core::providers::AuthMethod::ApiKey => "apikey".to_string(),
                    rustyclaw_core::providers::AuthMethod::DeviceFlow => "deviceflow".to_string(),
                    rustyclaw_core::providers::AuthMethod::None => "none".to_string(),
                    rustyclaw_core::providers::AuthMethod::OptionalApiKey => "apikey".to_string(),
                })
                .collect();
            let _ = gw_tx.send(GwEvent::ShowProviderSelector {
                providers,
                provider_ids: ids,
                auth_hints: hints,
            });
        }
        _ => {}
    }
    Ok(false)
}
