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
        ClientPayload::EngineList => handle_engine_list(writer, registry, configs).await,

        ClientPayload::EngineAction { engine, action } => {
            handle_engine_action(writer, registry, configs, engine, action).await
        }

        ClientPayload::EngineModelList { engine } => {
            handle_engine_model_list(writer, registry, configs, engine).await
        }

        ClientPayload::EngineModelPull {
            engine,
            model,
            expected_size_bytes,
        } => {
            handle_engine_model_pull(
                writer,
                registry,
                configs,
                engine,
                model,
                expected_size_bytes,
            )
            .await
        }

        ClientPayload::EngineModelAction {
            engine,
            model,
            action,
            context_length,
            extra_args,
        } => {
            handle_engine_model_action(
                writer,
                registry,
                configs,
                engine,
                model,
                action,
                context_length,
                extra_args,
            )
            .await
        }

        ClientPayload::EngineConfigSet { engine, config: _ } => {
            // Config persistence is handled by the caller (gateway main loop
            // updates Config.engines and calls cfg.save()). Here we just ack.
            send_action_result(writer, engine, None, true, "Configuration updated".into()).await
        }

        _ => Ok(()),
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Build and send an `EngineActionResult` frame.
async fn send_action_result(
    writer: &mut dyn TransportWriter,
    engine: String,
    model: Option<String>,
    ok: bool,
    message: String,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::EngineActionResult,
        payload: ServerPayload::EngineActionResult {
            engine,
            model,
            ok,
            message,
        },
    };
    send_frame(writer, &frame).await
}

// ── Sub-handlers ────────────────────────────────────────────────────────────

async fn handle_engine_list(
    writer: &mut dyn TransportWriter,
    registry: &EngineRegistry,
    configs: &HashMap<String, EngineConfig>,
) -> Result<()> {
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
            EngineRunStatus::Unhealthy { endpoint, .. } => (false, Some(endpoint.clone()), 0, 0),
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
    send_frame(writer, &frame).await
}

async fn handle_engine_action(
    writer: &mut dyn TransportWriter,
    registry: &EngineRegistry,
    configs: &HashMap<String, EngineConfig>,
    engine: String,
    action: String,
) -> Result<()> {
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
            (false, format!("{e:#}"))
        }
    };
    send_action_result(writer, engine, None, ok, message).await
}

async fn handle_engine_model_list(
    writer: &mut dyn TransportWriter,
    registry: &EngineRegistry,
    configs: &HashMap<String, EngineConfig>,
    engine: String,
) -> Result<()> {
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
    send_frame(writer, &frame).await
}

async fn handle_engine_model_pull(
    writer: &mut dyn TransportWriter,
    registry: &EngineRegistry,
    configs: &HashMap<String, EngineConfig>,
    engine: String,
    model: String,
    expected_size_bytes: Option<u64>,
) -> Result<()> {
    let cfg = configs.get(&engine).cloned().unwrap_or_default();

    // Disk space pre-flight check.
    if let Some(expected) = expected_size_bytes {
        if let Err(e) = rustyclaw_core::engines::preflight_disk_check(expected) {
            warn!(engine = %engine, model = %model, error = ?e, "Disk space pre-flight check failed");
            return send_action_result(writer, engine, Some(model), false, format!("{e:#}")).await;
        }
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);

    let Some(eng) = registry.get(&engine) else {
        drop(rx);
        return send_action_result(
            writer,
            engine.clone(),
            Some(model),
            false,
            format!("Unknown engine: {engine}"),
        )
        .await;
    };

    // Send initial progress.
    let frame = ServerFrame {
        frame_type: ServerFrameType::EnginePullProgress,
        payload: ServerPayload::EnginePullProgress {
            engine: engine.clone(),
            model: model.clone(),
            percent: 0.0,
            downloaded_bytes: 0,
            total_bytes: 0,
            status: "starting".into(),
        },
    };
    send_frame(writer, &frame).await?;

    // Drive the pull while draining the progress channel and forwarding
    // each update to the client. The channel MUST be drained concurrently:
    // engines await `send` on the sink, so a full, unread channel would
    // stall the pull indefinitely.
    let result = {
        let mut pull_fut = std::pin::pin!(eng.pull(&model, &cfg, Some(tx)));
        let mut rx_open = true;
        loop {
            tokio::select! {
                progress = rx.recv(), if rx_open => {
                    match progress {
                        Some(p) => {
                            let frame = ServerFrame {
                                frame_type: ServerFrameType::EnginePullProgress,
                                payload: ServerPayload::EnginePullProgress {
                                    engine: engine.clone(),
                                    model: model.clone(),
                                    percent: p.percent,
                                    downloaded_bytes: p.downloaded_bytes,
                                    total_bytes: p.total_bytes,
                                    status: p.status,
                                },
                            };
                            send_frame(writer, &frame).await?;
                        }
                        // Sink dropped inside pull — stop polling the channel
                        // and just wait for the pull future to finish.
                        None => rx_open = false,
                    }
                }
                res = &mut pull_fut => break res,
            }
        }
    };
    // Forward any progress updates still buffered after completion.
    while let Ok(p) = rx.try_recv() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::EnginePullProgress,
            payload: ServerPayload::EnginePullProgress {
                engine: engine.clone(),
                model: model.clone(),
                percent: p.percent,
                downloaded_bytes: p.downloaded_bytes,
                total_bytes: p.total_bytes,
                status: p.status,
            },
        };
        send_frame(writer, &frame).await?;
    }

    let (ok, message) = match result {
        Ok(msg) => (true, msg),
        Err(e) => {
            warn!(engine = %engine, model = %model, error = ?e, "Engine pull failed");
            (false, format!("{e:#}"))
        }
    };
    send_action_result(writer, engine, Some(model), ok, message).await
}

async fn handle_engine_model_action(
    writer: &mut dyn TransportWriter,
    registry: &EngineRegistry,
    configs: &HashMap<String, EngineConfig>,
    engine: String,
    model: String,
    action: String,
    context_length: Option<u32>,
    extra_args: Vec<String>,
) -> Result<()> {
    let mut cfg = configs.get(&engine).cloned().unwrap_or_default();

    // Apply per-model knobs to the config for this operation.
    if let Some(ctx) = context_length {
        match engine.as_str() {
            "ollama" => cfg.extra_args.push(format!("--num-ctx={ctx}")),
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
            (false, format!("{e:#}"))
        }
    };
    send_action_result(writer, engine, Some(model), ok, message).await
}
