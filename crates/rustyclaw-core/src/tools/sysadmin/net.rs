//! Network information and scanning tools.

use super::{sh, sh_async, which_first_async};
use serde_json::{Value, json};
use std::path::Path;
use tracing::{debug, instrument};

// ── net_info async ──────────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub async fn exec_net_info_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing action")?;
    tracing::Span::current().record("action", action);
    let target = args.get("target").and_then(|v| v.as_str());
    debug!(target, "Network info request");

    match action {
        "interfaces" => {
            let output = if cfg!(target_os = "macos") {
                sh_async("ifconfig | grep -E '^[a-z]|inet ' | head -60").await?
            } else {
                sh_async("ip -brief addr show 2>/dev/null || ifconfig 2>/dev/null | head -60")
                    .await?
            };
            Ok(json!({ "action": "interfaces", "output": output }).to_string())
        }

        "connections" => {
            let filter = target.unwrap_or("");
            let cmd = if cfg!(target_os = "macos") {
                if filter.is_empty() {
                    "netstat -an | head -60".to_string()
                } else {
                    format!("netstat -an | grep -i '{}' | head -60", filter)
                }
            } else if filter.is_empty() {
                "ss -tunapl 2>/dev/null | head -60 || netstat -tunapl 2>/dev/null | head -60"
                    .to_string()
            } else {
                format!("ss -tunapl 2>/dev/null | grep -i '{}' | head -60", filter)
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "connections", "filter": filter, "output": output }).to_string())
        }

        "routing" => {
            let output = if cfg!(target_os = "macos") {
                sh_async("netstat -rn | head -40").await?
            } else {
                sh_async("ip route show 2>/dev/null || netstat -rn 2>/dev/null | head -40").await?
            };
            Ok(json!({ "action": "routing", "output": output }).to_string())
        }

        "dns" => {
            let host = target.unwrap_or("example.com");
            let tool = which_first_async(&["dig", "nslookup", "host"]).await;
            let output = match tool.as_deref() {
                Some("dig") => sh_async(&format!("dig {} +short", host)).await?,
                Some("nslookup") => sh_async(&format!("nslookup {} 2>/dev/null", host)).await?,
                Some("host") => sh_async(&format!("host {}", host)).await?,
                _ => return Err("No DNS tool found".to_string()),
            };
            Ok(json!({ "action": "dns", "host": host, "tool": tool.unwrap_or_default(), "output": output }).to_string())
        }

        "ping" => {
            let host = target.ok_or("Missing 'target' for ping")?;
            let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(4);
            let output = sh_async(&format!("ping -c {} {} 2>&1", count, host)).await?;
            Ok(
                json!({ "action": "ping", "target": host, "count": count, "output": output })
                    .to_string(),
            )
        }

        "traceroute" => {
            let host = target.ok_or("Missing 'target' for traceroute")?;
            let tool = which_first_async(&["traceroute", "tracepath", "mtr"]).await;
            let cmd = match tool.as_deref() {
                Some("mtr") => format!("mtr -r -c 3 {} 2>&1", host),
                Some("tracepath") => format!("tracepath {} 2>&1 | head -30", host),
                Some("traceroute") => format!("traceroute -m 20 {} 2>&1", host),
                _ => return Err("No traceroute tool found".to_string()),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "traceroute", "target": host, "tool": tool.unwrap_or_default(), "output": output }).to_string())
        }

        "whois" => {
            let domain = target.ok_or("Missing 'target' for whois")?;
            let output = sh_async(&format!("whois {} 2>&1 | head -80", domain)).await?;
            Ok(json!({ "action": "whois", "target": domain, "output": output }).to_string())
        }

        "arp" => {
            let output = sh_async("arp -a 2>/dev/null | head -50").await?;
            Ok(json!({ "action": "arp", "output": output }).to_string())
        }

        "public_ip" => {
            let output = sh_async(
                "curl -s --max-time 5 ifconfig.me 2>/dev/null || \
                curl -s --max-time 5 api.ipify.org 2>/dev/null",
            )
            .await?;
            Ok(json!({ "action": "public_ip", "ip": output.trim() }).to_string())
        }

        "wifi" => {
            let output = if cfg!(target_os = "macos") {
                sh_async("system_profiler SPAirPortDataType 2>/dev/null | head -40").await?
            } else {
                sh_async("iwconfig 2>/dev/null || nmcli device wifi list 2>/dev/null | head -30")
                    .await?
            };
            Ok(json!({ "action": "wifi", "output": output }).to_string())
        }

        "bandwidth" => {
            let tool = which_first_async(&["speedtest-cli", "speedtest", "fast"]).await;
            match tool.as_deref() {
                Some(t) => {
                    let output = sh_async(&format!("{} --simple 2>&1 || {} 2>&1", t, t)).await?;
                    Ok(json!({ "action": "bandwidth", "tool": t, "output": output }).to_string())
                }
                None => Err("No bandwidth test tool found".to_string()),
            }
        }

        _ => Err(format!("Unknown action: {}", action)),
    }
}

// ── net_scan async ──────────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub async fn exec_net_scan_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing action")?;
    tracing::Span::current().record("action", action);
    let target = args.get("target").and_then(|v| v.as_str());
    debug!(target, "Network scan request");

    match action {
        "nmap" => {
            let host = target.ok_or("Missing 'target' for nmap")?;
            let scan_type = args
                .get("scan_type")
                .and_then(|v| v.as_str())
                .unwrap_or("quick");
            let ports = args.get("ports").and_then(|v| v.as_str());

            if which_first_async(&["nmap"]).await.is_none() {
                return Err("nmap is not installed".to_string());
            }

            let cmd = match scan_type {
                "quick" => {
                    if let Some(p) = ports {
                        format!("nmap -T4 -p {} {} 2>&1", p, host)
                    } else {
                        format!("nmap -T4 -F {} 2>&1", host)
                    }
                }
                "full" => format!("nmap -T4 -p- {} 2>&1", host),
                "service" | "version" => {
                    if let Some(p) = ports {
                        format!("nmap -sV -p {} {} 2>&1", p, host)
                    } else {
                        format!("nmap -sV -F {} 2>&1", host)
                    }
                }
                "os" => format!("sudo nmap -O {} 2>&1", host),
                "udp" => {
                    if let Some(p) = ports {
                        format!("sudo nmap -sU -p {} {} 2>&1", p, host)
                    } else {
                        format!("sudo nmap -sU --top-ports 20 {} 2>&1", host)
                    }
                }
                "vuln" => format!("nmap --script vuln {} 2>&1", host),
                "ping" => format!("nmap -sn {} 2>&1", host),
                "stealth" => format!("sudo nmap -sS -T2 {} 2>&1", host),
                _ => return Err(format!("Unknown scan_type: {}", scan_type)),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "nmap", "target": host, "scan_type": scan_type, "output": output }).to_string())
        }

        "tcpdump" => {
            let iface = args
                .get("interface")
                .and_then(|v| v.as_str())
                .unwrap_or("any");
            let filter = target.unwrap_or("");
            let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(20);

            if which_first_async(&["tcpdump"]).await.is_none() {
                return Err("tcpdump is not installed".to_string());
            }

            let cmd = if filter.is_empty() {
                format!("sudo timeout 10 tcpdump -i {} -c {} -nn 2>&1", iface, count)
            } else {
                format!(
                    "sudo timeout 10 tcpdump -i {} -c {} -nn '{}' 2>&1",
                    iface, count, filter
                )
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "tcpdump", "interface": iface, "filter": filter, "count": count, "output": output }).to_string())
        }

        "port_check" => {
            let host = target.ok_or("Missing 'target' for port_check")?;
            let port = args.get("port").and_then(|v| v.as_u64()).or_else(|| {
                args.get("ports")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
            });

            if let Some(p) = port {
                let cmd = format!(
                    "nc -z -w 3 {} {} 2>&1 && echo 'OPEN' || echo 'CLOSED'",
                    host, p
                );
                let output = sh_async(&cmd).await?;
                Ok(json!({
                    "action": "port_check", "target": host, "port": p,
                    "status": if output.contains("OPEN") { "open" } else { "closed" },
                    "output": output
                })
                .to_string())
            } else {
                let cmd = format!(
                    "for p in 22 80 443 8080 3306 5432 6379 27017 3000 8443; do \
                       nc -z -w 1 {} $p 2>/dev/null && echo \"$p OPEN\" || echo \"$p CLOSED\"; done",
                    host
                );
                let output = sh_async(&cmd).await?;
                Ok(json!({ "action": "port_check", "target": host, "output": output }).to_string())
            }
        }

        "listen" => {
            let output = if cfg!(target_os = "macos") {
                sh_async("lsof -i -P -n | grep LISTEN | head -40").await?
            } else {
                sh_async("ss -tlnp 2>/dev/null | head -40 || netstat -tlnp 2>/dev/null | head -40")
                    .await?
            };
            Ok(json!({ "action": "listen", "output": output }).to_string())
        }

        "sniff" => {
            let iface = args
                .get("interface")
                .and_then(|v| v.as_str())
                .unwrap_or("any");
            let seconds = args.get("seconds").and_then(|v| v.as_u64()).unwrap_or(5);
            let tool = which_first_async(&["tcpdump", "tshark"]).await;
            let cmd = match tool.as_deref() {
                Some("tcpdump") => format!(
                    "sudo timeout {} tcpdump -i {} -c 30 -q -nn 2>&1",
                    seconds, iface
                ),
                Some("tshark") => format!("timeout {} tshark -i {} -c 30 -q 2>&1", seconds, iface),
                _ => return Err("No packet capture tool found".to_string()),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "sniff", "interface": iface, "seconds": seconds, "tool": tool.unwrap_or_default(), "output": output }).to_string())
        }

        "discover" => {
            let subnet = target.unwrap_or("192.168.1.0/24");
            let tool = which_first_async(&["nmap", "arp-scan", "fping"]).await;
            let cmd = match tool.as_deref() {
                Some("nmap") => format!("nmap -sn {} 2>&1", subnet),
                Some("arp-scan") => format!("sudo arp-scan {} 2>&1 | head -40", subnet),
                Some("fping") => format!("fping -a -g {} 2>/dev/null | head -40", subnet),
                _ => "arp -a 2>/dev/null | head -40".to_string(),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "discover", "subnet": subnet, "tool": tool.unwrap_or_else(|| "arp".into()), "output": output }).to_string())
        }

        _ => Err(format!("Unknown action: {}", action)),
    }
}

// ── Sync implementations ────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_net_info(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing action")?;
    tracing::Span::current().record("action", action);
    let target = args.get("target").and_then(|v| v.as_str());

    match action {
        "interfaces" => {
            let output = if cfg!(target_os = "macos") {
                sh("ifconfig | grep -E '^[a-z]|inet ' | head -60")?
            } else {
                sh("ip -brief addr show 2>/dev/null || ifconfig 2>/dev/null | head -60")?
            };
            Ok(json!({ "action": "interfaces", "output": output }).to_string())
        }
        "ping" => {
            let host = target.ok_or("Missing target")?;
            let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(4);
            let output = sh(&format!("ping -c {} {} 2>&1", count, host))?;
            Ok(json!({ "action": "ping", "target": host, "output": output }).to_string())
        }
        "public_ip" => {
            let output = sh("curl -s --max-time 5 ifconfig.me 2>/dev/null")?;
            Ok(json!({ "action": "public_ip", "ip": output.trim() }).to_string())
        }
        _ => Err(format!(
            "Sync not fully supported for '{}'. Use async.",
            action
        )),
    }
}

#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_net_scan(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing action")?;
    tracing::Span::current().record("action", action);
    let target = args.get("target").and_then(|v| v.as_str());

    match action {
        "listen" => {
            let output = if cfg!(target_os = "macos") {
                sh("lsof -i -P -n | grep LISTEN | head -40")?
            } else {
                sh("ss -tlnp 2>/dev/null | head -40")?
            };
            Ok(json!({ "action": "listen", "output": output }).to_string())
        }
        "port_check" => {
            let host = target.ok_or("Missing target")?;
            let port = args.get("port").and_then(|v| v.as_u64()).unwrap_or(80);
            let cmd = format!(
                "nc -z -w 3 {} {} 2>&1 && echo 'OPEN' || echo 'CLOSED'",
                host, port
            );
            let output = sh(&cmd)?;
            Ok(json!({ "action": "port_check", "target": host, "port": port, "status": if output.contains("OPEN") { "open" } else { "closed" } }).to_string())
        }
        _ => Err(format!(
            "Sync not fully supported for '{}'. Use async.",
            action
        )),
    }
}
