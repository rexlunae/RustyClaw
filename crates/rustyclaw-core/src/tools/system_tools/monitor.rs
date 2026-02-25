//! System monitoring: CPU, memory, disk, network stats; battery health.

use super::{sh, sh_async};
use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, instrument};

// ── Async implementations ───────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir))]
pub async fn exec_system_monitor_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let metric = args.get("metric").and_then(|v| v.as_str()).unwrap_or("all");
    debug!(metric, "System monitor request");

    let mut result = serde_json::Map::new();

    if metric == "all" || metric == "cpu" {
        let load = sh_async("sysctl -n vm.loadavg 2>/dev/null || cat /proc/loadavg 2>/dev/null").await.unwrap_or_default();
        result.insert("load_average".into(), json!(load.trim()));
        let top_cpu = sh_async("ps aux --sort=-%cpu 2>/dev/null | head -11 || ps aux -r | head -11").await.unwrap_or_default();
        result.insert("top_cpu_processes".into(), json!(top_cpu.trim()));
    }

    if metric == "all" || metric == "memory" {
        let mem = sh_async("vm_stat 2>/dev/null | head -10 || free -h 2>/dev/null").await.unwrap_or_default();
        result.insert("memory".into(), json!(mem.trim()));
        let top_mem = sh_async("ps aux --sort=-%mem 2>/dev/null | head -11 || ps aux -m | head -11").await.unwrap_or_default();
        result.insert("top_memory_processes".into(), json!(top_mem.trim()));
    }

    if metric == "all" || metric == "disk" {
        let df = sh_async("df -h / 2>/dev/null").await.unwrap_or_default();
        result.insert("disk".into(), json!(df.trim()));
    }

    if metric == "all" || metric == "network" {
        let net = sh_async("netstat -ib 2>/dev/null | head -5 || ip -s link 2>/dev/null | head -20").await.unwrap_or_default();
        result.insert("network".into(), json!(net.trim()));
    }

    Ok(Value::Object(result).to_string())
}

#[instrument(skip(_args, _workspace_dir))]
pub async fn exec_battery_health_async(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let pmset = sh_async("pmset -g batt 2>/dev/null").await.unwrap_or_default();
    let ioreg = sh_async("ioreg -r -c AppleSmartBattery 2>/dev/null | grep -E '(CycleCount|MaxCapacity|DesignCapacity|Temperature|FullyCharged|IsCharging)' | head -10").await.unwrap_or_default();
    let linux = sh_async("cat /sys/class/power_supply/BAT0/status 2>/dev/null && cat /sys/class/power_supply/BAT0/capacity 2>/dev/null && cat /sys/class/power_supply/BAT0/cycle_count 2>/dev/null").await.unwrap_or_default();

    if pmset.trim().is_empty() && linux.trim().is_empty() {
        return Ok(json!({ "available": false, "note": "No battery detected." }).to_string());
    }

    let mut result = serde_json::Map::new();
    result.insert("available".into(), json!(true));
    if !pmset.trim().is_empty() { result.insert("pmset".into(), json!(pmset.trim())); }
    if !ioreg.trim().is_empty() { result.insert("battery_details".into(), json!(ioreg.trim())); }
    if !linux.trim().is_empty() { result.insert("linux_battery".into(), json!(linux.trim())); }

    Ok(Value::Object(result).to_string())
}

// ── Sync implementations ────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir))]
pub fn exec_system_monitor(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let metric = args.get("metric").and_then(|v| v.as_str()).unwrap_or("all");
    let mut result = serde_json::Map::new();

    if metric == "all" || metric == "cpu" {
        let load = sh("sysctl -n vm.loadavg 2>/dev/null || cat /proc/loadavg 2>/dev/null").unwrap_or_default();
        result.insert("load_average".into(), json!(load.trim()));
        let top_cpu = sh("ps aux --sort=-%cpu 2>/dev/null | head -11 || ps aux -r | head -11").unwrap_or_default();
        result.insert("top_cpu_processes".into(), json!(top_cpu.trim()));
    }

    if metric == "all" || metric == "memory" {
        let mem = sh("vm_stat 2>/dev/null | head -10 || free -h 2>/dev/null").unwrap_or_default();
        result.insert("memory".into(), json!(mem.trim()));
    }

    if metric == "all" || metric == "disk" {
        let df = sh("df -h / 2>/dev/null").unwrap_or_default();
        result.insert("disk".into(), json!(df.trim()));
    }

    Ok(Value::Object(result).to_string())
}

#[instrument(skip(_args, _workspace_dir))]
pub fn exec_battery_health(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let pmset = sh("pmset -g batt 2>/dev/null").unwrap_or_default();
    let linux = sh("cat /sys/class/power_supply/BAT0/status 2>/dev/null").unwrap_or_default();

    if pmset.trim().is_empty() && linux.trim().is_empty() {
        return Ok(json!({ "available": false, "note": "No battery detected." }).to_string());
    }

    let mut result = serde_json::Map::new();
    result.insert("available".into(), json!(true));
    if !pmset.trim().is_empty() { result.insert("pmset".into(), json!(pmset.trim())); }
    if !linux.trim().is_empty() { result.insert("linux_battery".into(), json!(linux.trim())); }

    Ok(Value::Object(result).to_string())
}
