//! Handlers for local engine management requests.
//!
//! Dispatches engine/model lifecycle operations to the [`EngineRegistry`].

use anyhow::Result;
use rustyclaw_core::engines::{EngineConfig, EngineRegistry, EngineRunStatus};
use rustyclaw_core::gateway::TransportWriter;
use rustyclaw_core::gateway::protocol::frames::*;
use rustyclaw_core::gateway::protocol::server::send_frame;
use std::collections::HashMap;
use tracing::warn;

/// Handle engine management client frames.
pub async fn handle_engine_request(
    writer: &mut dyn TransportWriter,
    payload: ClientPayload,
    registry: &EngineRegistry,
    configs: &HashMap<String, EngineConfig>,
) -> Result<()> {
    match payload {
        ClientPayload::EngineList => {
            let mut engines = Vec::new();
            for engine in registry.all() {
                let cfg = configs.get(engine.id()).cloned().unwrap_or_default();
                let status = engine.status(&cfg).await;
                let (running, endpoint, available, loaded) = match &status.run_status {
                    EngineRunStatus::Running {
                        endpoint,
                        loaded_models,
                        available_models,
                    } => (
                        true,
                        Some(endpoint.clone()),
                        *available_models,
                        *loaded_models,
                    ),
                    EngineRunStatus::Unhealthy { endpoint, .. } => {
                        (false, Some(endpoint.clone()), 0, 0)
                    }
                    EngineRunStatus::Stopped => (false, None, 0, 0),
                };
                engines.push(EngineInfoDto {
                    id: engine.id().to_string(),
                    display_name: engine.display_name().to_string(),
                    installed: status.presence.installed,
                    running,
                    version: status.presence.version,
                    endpoint,
                    available_models: available,
                    loaded_models: loaded,
                    capabilities: engine.capabilities().into(),
                });
            }
            let frame = ServerFrame {
                frame_type: ServerFrameType::EngineListResult,
                payload: ServerPayload::EngineListResult { engines },
            };
            send_frame(writer, &frame).await?;
        }

        ClientPayload::EngineAction { engine, action } => {
            let cfg = configs.get(&engine).cloned().unwrap_or_default();
            let result = if let Some(eng) = registry.get(&engine) {
                match action.as_str() {
                    "install" => eng.install(None).await,
                    "start" => eng.start(&cfg).await,
                    "stop" => eng.stop().await,
                    _ => Err(anyhow::anyhow!("Unknown engine action: {}", action)),
                }
            } else {
                Err(anyhow::anyhow!("Unknown engine: {}", engine))
            };
            let (ok, message) = match result {
                Ok(msg) => (true, msg),
                Err(e) => {
                    warn!(engine = %engine, action = %action, error = ?e, "Engine action failed");
                    (false, format!("{e}"))
                }
            };
            let frame = ServerFrame {
                frame_type: ServerFrameType::EngineActionResult,
                payload: ServerPayload::EngineActionResult {
                    engine,
                    model: None,
                    ok,
                    message,
                },
            };
            send_frame(writer, &frame).await?;
        }

        ClientPayload::EngineModelList { engine } => {
            let cfg = configs.get(&engine).cloned().unwrap_or_default();
            let models = if let Some(eng) = registry.get(&engine) {
                match eng.list_models(&cfg).await {
                    Ok(models) => models.into_iter().map(EngineModelDto::from).collect(),
                    Err(_) => vec![],
                }
            } else {
                vec![]
            };
            let frame = ServerFrame {
                frame_type: ServerFrameType::EngineModelListResult,
                payload: ServerPayload::EngineModelListResult { engine, models },
            };
            send_frame(writer, &frame).await?;
        }

        ClientPayload::EngineModelPull {
            engine,
            model,
            expected_size_bytes,
        } => {
            let cfg = configs.get(&engine).cloned().unwrap_or_default();

            // P6: Disk space pre-flight check.
            if let Some(expected) = expected_size_bytes {
                if let Err(e) = rustyclaw_core::engines::preflight_disk_check(expected) {
                    warn!(engine = %engine, model = %model, error = ?e, "Disk space pre-flight check failed");
                    let frame = ServerFrame {
                        frame_type: ServerFrameType::EngineActionResult,
                        payload: ServerPayload::EngineActionResult {
                            engine,
                            model: Some(model),
                            ok: false,
                            message: format!("{e}"),
                        },
                    };
                    send_frame(writer, &frame).await?;
                    return Ok(());
                }
            }

            let (tx, rx) = tokio::sync::mpsc::channel(32);

            // Spawn the pull in background and stream progress
            let eng_id = engine.clone();
            let model_clone = model.clone();
            if let Some(eng) = registry.get(&engine) {
                let cfg_clone = cfg.clone();

                // Send initial progress
                let frame = ServerFrame {
                    frame_type: ServerFrameType::EnginePullProgress,
                    payload: ServerPayload::EnginePullProgress {
                        engine: eng_id.clone(),
                        model: model_clone.clone(),
                        percent: 0.0,
                        downloaded_bytes: 0,
                        total_bytes: 0,
                        status: "starting".into(),
                    },
                };
                send_frame(writer, &frame).await?;

                // Pull synchronously (streamed progress TBD with background task)
                let result = eng.pull(&model_clone, &cfg_clone, Some(tx)).await;
                drop(rx);

                let (ok, message) = match result {
                    Ok(msg) => (true, msg),
                    Err(e) => {
                        warn!(engine = %eng_id, model = %model_clone, error = ?e, "Engine pull failed");
                        (false, format!("{e}"))
                    }
                };
                let frame = ServerFrame {
                    frame_type: ServerFrameType::EngineActionResult,
                    payload: ServerPayload::EngineActionResult {
                        engine: eng_id,
                        model: Some(model_clone),
                        ok,
                        message,
                    },
                };
                send_frame(writer, &frame).await?;
            } else {
                drop(rx);
                let frame = ServerFrame {
                    frame_type: ServerFrameType::EngineActionResult,
                    payload: ServerPayload::EngineActionResult {
                        engine: eng_id,
                        model: Some(model_clone),
                        ok: false,
                        message: format!("Unknown engine: {}", engine),
                    },
                };
                send_frame(writer, &frame).await?;
            }
        }

        ClientPayload::EngineModelAction {
            engine,
            model,
            action,
            context_length,
            extra_args,
        } => {
            let mut cfg = configs.get(&engine).cloned().unwrap_or_default();
            // P6: Apply per-model knobs to the config for this operation.
            if let Some(ctx) = context_length {
                // Inject context length as extra arg (engine-specific).
                match engine.as_str() {
                    "ollama" => {
                        // Ollama uses num_ctx parameter at load time via API.
                        // We'll pass it as an extra_arg marker for the engine impl.
                        cfg.extra_args.push(format!("--num-ctx={}", ctx));
                    }
                    "llamacpp" => {
                        cfg.extra_args.push("--ctx-size".to_string());
                        cfg.extra_args.push(ctx.to_string());
                    }
                    _ => {}
                }
            }
            if !extra_args.is_empty() {
                cfg.extra_args.extend(extra_args);
            }

            let result = if let Some(eng) = registry.get(&engine) {
                match action.as_str() {
                    "load" => eng.load(&model, &cfg).await,
                    "unload" => eng.unload(&model, &cfg).await,
                    "remove" => eng.remove(&model, &cfg).await,
                    _ => Err(anyhow::anyhow!("Unknown model action: {}", action)),
                }
            } else {
                Err(anyhow::anyhow!("Unknown engine: {}", engine))
            };
            let (ok, message) = match result {
                Ok(msg) => (true, msg),
                Err(e) => {
                    warn!(engine = %engine, model = %model, action = %action, error = ?e, "Engine model action failed");
                    (false, format!("{e}"))
                }
            };
            let frame = ServerFrame {
                frame_type: ServerFrameType::EngineActionResult,
                payload: ServerPayload::EngineActionResult {
                    engine,
                    model: Some(model),
                    ok,
                    message,
                },
            };
            send_frame(writer, &frame).await?;
        }

        ClientPayload::EngineConfigSet { engine, config: _ } => {
            // Config persistence is handled by the caller (gateway main loop
            // updates Config.engines and calls cfg.save()). Here we just ack.
            let frame = ServerFrame {
                frame_type: ServerFrameType::EngineActionResult,
                payload: ServerPayload::EngineActionResult {
                    engine,
                    model: None,
                    ok: true,
                    message: "Configuration updated".into(),
                },
            };
            send_frame(writer, &frame).await?;
        }

        _ => {}
    }
    Ok(())
}
