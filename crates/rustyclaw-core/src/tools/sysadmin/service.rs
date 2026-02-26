//! Service management: start, stop, restart, enable, disable, logs.

use super::{detect_service_manager, detect_service_manager_async, sh, sh_async};
use serde_json::{Value, json};
use std::path::Path;
use tracing::{debug, instrument};

// ── Async implementation ────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub async fn exec_service_manage_async(
    args: &Value,
    _workspace_dir: &Path,
) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing action")?;
    tracing::Span::current().record("action", action);
    let service = args.get("service").and_then(|v| v.as_str());
    let init = detect_service_manager_async().await;
    debug!(service, init_system = init, "Service management request");

    match action {
        "list" => {
            let filter = service.unwrap_or("");
            let cmd = match init {
                "systemd" => {
                    if filter.is_empty() {
                        "systemctl list-units --type=service --no-pager | head -50".to_string()
                    } else {
                        format!(
                            "systemctl list-units --type=service --no-pager | grep -i '{}' | head -30",
                            filter
                        )
                    }
                }
                "launchd" => {
                    if filter.is_empty() {
                        "launchctl list | head -50".to_string()
                    } else {
                        format!("launchctl list | grep -i '{}' | head -30", filter)
                    }
                }
                "sysvinit" => "ls /etc/init.d/ | head -50".to_string(),
                _ => return Err(format!("Unknown init system: {}", init)),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "list", "init_system": init, "filter": filter, "output": output }).to_string())
        }

        "status" => {
            let svc = service.ok_or("Missing 'service'")?;
            let cmd = match init {
                "systemd" => format!("systemctl status {} --no-pager 2>&1", svc),
                "launchd" => format!(
                    "launchctl print system/{} 2>&1 || launchctl list {} 2>&1",
                    svc, svc
                ),
                "sysvinit" => format!("/etc/init.d/{} status 2>&1", svc),
                _ => return Err(format!("Unknown init system: {}", init)),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "status", "service": svc, "init_system": init, "output": output }).to_string())
        }

        "start" => {
            let svc = service.ok_or("Missing 'service'")?;
            let cmd = match init {
                "systemd" => format!("sudo systemctl start {} 2>&1", svc),
                "launchd" => format!("sudo launchctl start {} 2>&1", svc),
                "sysvinit" => format!("sudo /etc/init.d/{} start 2>&1", svc),
                _ => return Err(format!("Unknown init system: {}", init)),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "start", "service": svc, "init_system": init, "output": if output.is_empty() { "Service started.".into() } else { output } }).to_string())
        }

        "stop" => {
            let svc = service.ok_or("Missing 'service'")?;
            let cmd = match init {
                "systemd" => format!("sudo systemctl stop {} 2>&1", svc),
                "launchd" => format!("sudo launchctl stop {} 2>&1", svc),
                "sysvinit" => format!("sudo /etc/init.d/{} stop 2>&1", svc),
                _ => return Err(format!("Unknown init system: {}", init)),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "stop", "service": svc, "init_system": init, "output": if output.is_empty() { "Service stopped.".into() } else { output } }).to_string())
        }

        "restart" => {
            let svc = service.ok_or("Missing 'service'")?;
            let cmd = match init {
                "systemd" => format!("sudo systemctl restart {} 2>&1", svc),
                "launchd" => format!("sudo launchctl kickstart -k system/{} 2>&1", svc),
                "sysvinit" => format!("sudo /etc/init.d/{} restart 2>&1", svc),
                _ => return Err(format!("Unknown init system: {}", init)),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "restart", "service": svc, "init_system": init, "output": if output.is_empty() { "Service restarted.".into() } else { output } }).to_string())
        }

        "enable" => {
            let svc = service.ok_or("Missing 'service'")?;
            let cmd = match init {
                "systemd" => format!("sudo systemctl enable {} 2>&1", svc),
                "launchd" => format!(
                    "sudo launchctl load -w /Library/LaunchDaemons/{}.plist 2>&1",
                    svc
                ),
                _ => return Err("enable requires systemd or launchd".to_string()),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "enable", "service": svc, "init_system": init, "output": if output.is_empty() { "Service enabled.".into() } else { output } }).to_string())
        }

        "disable" => {
            let svc = service.ok_or("Missing 'service'")?;
            let cmd = match init {
                "systemd" => format!("sudo systemctl disable {} 2>&1", svc),
                "launchd" => format!(
                    "sudo launchctl unload -w /Library/LaunchDaemons/{}.plist 2>&1",
                    svc
                ),
                _ => return Err("disable requires systemd or launchd".to_string()),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "disable", "service": svc, "init_system": init, "output": if output.is_empty() { "Service disabled.".into() } else { output } }).to_string())
        }

        "logs" => {
            let svc = service.ok_or("Missing 'service'")?;
            let lines = args.get("lines").and_then(|v| v.as_u64()).unwrap_or(50);
            let cmd = match init {
                "systemd" => format!("journalctl -u {} -n {} --no-pager 2>&1", svc, lines),
                "launchd" => format!(
                    "log show --predicate 'subsystem==\"{}\"' --last 5m --style compact 2>&1 | tail -{}",
                    svc, lines
                ),
                _ => return Err("Service logs require systemd or launchd".to_string()),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "logs", "service": svc, "lines": lines, "init_system": init, "output": output }).to_string())
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: list, status, start, stop, restart, enable, disable, logs",
            action
        )),
    }
}

// ── Sync implementation ─────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_service_manage(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing action")?;
    tracing::Span::current().record("action", action);
    let service = args.get("service").and_then(|v| v.as_str());
    let init = detect_service_manager();

    match action {
        "list" => {
            let cmd = match init {
                "systemd" => "systemctl list-units --type=service --no-pager | head -50",
                "launchd" => "launchctl list | head -50",
                _ => return Err(format!("Unknown init: {}", init)),
            };
            let output = sh(cmd)?;
            Ok(json!({ "action": "list", "init_system": init, "output": output }).to_string())
        }
        "status" => {
            let svc = service.ok_or("Missing service")?;
            let cmd = match init {
                "systemd" => format!("systemctl status {} --no-pager 2>&1", svc),
                "launchd" => format!("launchctl list {} 2>&1", svc),
                _ => return Err(format!("Unknown init: {}", init)),
            };
            let output = sh(&cmd)?;
            Ok(json!({ "action": "status", "service": svc, "output": output }).to_string())
        }
        _ => Err(format!(
            "Sync not fully supported for '{}'. Use async.",
            action
        )),
    }
}
