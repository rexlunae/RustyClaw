//! Firewall management: status, rules, allow/deny ports, enable/disable.

use super::{sh, sh_async, which_first, which_first_async};
use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, instrument};

// ── Helpers ─────────────────────────────────────────────────────────────────

fn detect_firewall_backend() -> &'static str {
    if cfg!(target_os = "macos") {
        "pf"
    } else if which_first(&["ufw"]).is_some() {
        "ufw"
    } else if which_first(&["firewall-cmd"]).is_some() {
        "firewalld"
    } else if which_first(&["iptables"]).is_some() {
        "iptables"
    } else if which_first(&["nft"]).is_some() {
        "nftables"
    } else {
        "unknown"
    }
}

async fn detect_firewall_backend_async() -> &'static str {
    if cfg!(target_os = "macos") {
        return "pf";
    }
    if which_first_async(&["ufw"]).await.is_some() {
        return "ufw";
    }
    if which_first_async(&["firewall-cmd"]).await.is_some() {
        return "firewalld";
    }
    if which_first_async(&["iptables"]).await.is_some() {
        return "iptables";
    }
    if which_first_async(&["nft"]).await.is_some() {
        return "nftables";
    }
    "unknown"
}

// ── Async implementation ────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub async fn exec_firewall_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args.get("action").and_then(|v| v.as_str()).ok_or("Missing action")?;
    tracing::Span::current().record("action", action);
    let backend = detect_firewall_backend_async().await;
    debug!(backend, "Firewall management request");

    match action {
        "status" => {
            let cmd = match backend {
                "pf" => "sudo pfctl -s info 2>&1 | head -10",
                "ufw" => "sudo ufw status verbose 2>&1",
                "firewalld" => "sudo firewall-cmd --state 2>&1 && sudo firewall-cmd --list-all 2>&1",
                "iptables" => "sudo iptables -L -n --line-numbers 2>&1 | head -50",
                "nftables" => "sudo nft list ruleset 2>&1 | head -50",
                _ => return Err("No supported firewall found".to_string()),
            };
            let output = sh_async(cmd).await?;
            Ok(json!({ "action": "status", "backend": backend, "output": output }).to_string())
        }

        "rules" => {
            let cmd = match backend {
                "pf" => "sudo pfctl -s rules 2>&1",
                "ufw" => "sudo ufw status numbered 2>&1",
                "firewalld" => "sudo firewall-cmd --list-all 2>&1",
                "iptables" => "sudo iptables -L -n -v --line-numbers 2>&1 | head -60",
                "nftables" => "sudo nft list ruleset 2>&1 | head -60",
                _ => return Err("No supported firewall found".to_string()),
            };
            let output = sh_async(cmd).await?;
            Ok(json!({ "action": "rules", "backend": backend, "output": output }).to_string())
        }

        "allow" => {
            let port = args.get("port").and_then(|v| v.as_u64())
                .or_else(|| args.get("port").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()))
                .ok_or("Missing 'port'")?;
            let proto = args.get("protocol").and_then(|v| v.as_str()).unwrap_or("tcp");

            let cmd = match backend {
                "ufw" => format!("sudo ufw allow {}/{} 2>&1", port, proto),
                "firewalld" => format!("sudo firewall-cmd --add-port={}/{} --permanent 2>&1 && sudo firewall-cmd --reload 2>&1", port, proto),
                "iptables" => format!("sudo iptables -A INPUT -p {} --dport {} -j ACCEPT 2>&1", proto, port),
                "pf" => format!("echo 'pass in proto {} from any to any port {}' | sudo pfctl -f - 2>&1", proto, port),
                _ => return Err("No supported firewall found".to_string()),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "allow", "port": port, "protocol": proto, "backend": backend, "output": if output.is_empty() { format!("Port {}/{} allowed.", port, proto) } else { output } }).to_string())
        }

        "deny" => {
            let port = args.get("port").and_then(|v| v.as_u64())
                .or_else(|| args.get("port").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()))
                .ok_or("Missing 'port'")?;
            let proto = args.get("protocol").and_then(|v| v.as_str()).unwrap_or("tcp");

            let cmd = match backend {
                "ufw" => format!("sudo ufw deny {}/{} 2>&1", port, proto),
                "firewalld" => format!("sudo firewall-cmd --remove-port={}/{} --permanent 2>&1 && sudo firewall-cmd --reload 2>&1", port, proto),
                "iptables" => format!("sudo iptables -A INPUT -p {} --dport {} -j DROP 2>&1", proto, port),
                _ => return Err("No supported firewall found".to_string()),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "deny", "port": port, "protocol": proto, "backend": backend, "output": if output.is_empty() { format!("Port {}/{} denied.", port, proto) } else { output } }).to_string())
        }

        "enable" => {
            let cmd = match backend {
                "ufw" => "sudo ufw --force enable 2>&1",
                "firewalld" => "sudo systemctl start firewalld && sudo systemctl enable firewalld 2>&1",
                "pf" => "sudo pfctl -e 2>&1",
                _ => return Err("No supported firewall found".to_string()),
            };
            let output = sh_async(cmd).await?;
            Ok(json!({ "action": "enable", "backend": backend, "output": if output.is_empty() { "Firewall enabled.".into() } else { output } }).to_string())
        }

        "disable" => {
            let cmd = match backend {
                "ufw" => "sudo ufw disable 2>&1",
                "firewalld" => "sudo systemctl stop firewalld 2>&1",
                "pf" => "sudo pfctl -d 2>&1",
                _ => return Err("No supported firewall found".to_string()),
            };
            let output = sh_async(cmd).await?;
            Ok(json!({ "action": "disable", "backend": backend, "output": if output.is_empty() { "Firewall disabled.".into() } else { output } }).to_string())
        }

        _ => Err(format!("Unknown action: {}. Valid: status, rules, allow, deny, enable, disable", action)),
    }
}

// ── Sync implementation ─────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_firewall(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args.get("action").and_then(|v| v.as_str()).ok_or("Missing action")?;
    tracing::Span::current().record("action", action);
    let backend = detect_firewall_backend();

    match action {
        "status" => {
            let cmd = match backend {
                "pf" => "sudo pfctl -s info 2>&1 | head -10",
                "ufw" => "sudo ufw status verbose 2>&1",
                "iptables" => "sudo iptables -L -n --line-numbers 2>&1 | head -50",
                _ => return Err("No supported firewall found".to_string()),
            };
            let output = sh(cmd)?;
            Ok(json!({ "action": "status", "backend": backend, "output": output }).to_string())
        }
        "rules" => {
            let cmd = match backend {
                "ufw" => "sudo ufw status numbered 2>&1",
                "iptables" => "sudo iptables -L -n -v --line-numbers 2>&1 | head -60",
                _ => return Err("No supported firewall found".to_string()),
            };
            let output = sh(cmd)?;
            Ok(json!({ "action": "rules", "backend": backend, "output": output }).to_string())
        }
        _ => Err(format!("Sync not fully supported for '{}'. Use async.", action)),
    }
}
