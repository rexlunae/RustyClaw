//! Package management: install, uninstall, upgrade, search, list, info.

use super::{sh, sh_async, detect_pkg_manager, detect_pkg_manager_async};
use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, warn, instrument};

// ── Async implementation ────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub async fn exec_pkg_manage_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: action")?;

    tracing::Span::current().record("action", action);

    let package = args.get("package").and_then(|v| v.as_str());
    let manager_override = args.get("manager").and_then(|v| v.as_str());

    debug!(package, manager = manager_override, "Package management request");

    let (mgr, mgr_name) = if let Some(m) = manager_override {
        (m, m)
    } else {
        let (m, n) = detect_pkg_manager_async().await;
        if m.is_empty() {
            warn!("No supported package manager found");
            return Err("No supported package manager found on this system".to_string());
        }
        (m, n)
    };

    debug!(manager = mgr_name, "Using package manager");

    match action {
        "install" => {
            let pkg = package.ok_or("Missing 'package' for install action")?;
            let cmd = match mgr {
                "brew" => format!("brew install {}", pkg),
                "apt" | "apt-get" => format!("sudo apt-get install -y {}", pkg),
                "dnf" => format!("sudo dnf install -y {}", pkg),
                "yum" => format!("sudo yum install -y {}", pkg),
                "pacman" => format!("sudo pacman -S --noconfirm {}", pkg),
                "zypper" => format!("sudo zypper install -y {}", pkg),
                "apk" => format!("sudo apk add {}", pkg),
                "snap" => format!("sudo snap install {}", pkg),
                "flatpak" => format!("flatpak install -y {}", pkg),
                "port" => format!("sudo port install {}", pkg),
                "nix-env" => format!("nix-env -iA nixpkgs.{}", pkg),
                _ => return Err(format!("Unknown package manager: {}", mgr)),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "install", "package": pkg, "manager": mgr_name, "output": output }).to_string())
        }

        "uninstall" | "remove" => {
            let pkg = package.ok_or("Missing 'package' for uninstall action")?;
            let cmd = match mgr {
                "brew" => format!("brew uninstall {}", pkg),
                "apt" | "apt-get" => format!("sudo apt-get remove -y {}", pkg),
                "dnf" => format!("sudo dnf remove -y {}", pkg),
                "yum" => format!("sudo yum remove -y {}", pkg),
                "pacman" => format!("sudo pacman -R --noconfirm {}", pkg),
                "zypper" => format!("sudo zypper remove -y {}", pkg),
                "apk" => format!("sudo apk del {}", pkg),
                "snap" => format!("sudo snap remove {}", pkg),
                "flatpak" => format!("flatpak uninstall -y {}", pkg),
                "port" => format!("sudo port uninstall {}", pkg),
                "nix-env" => format!("nix-env -e {}", pkg),
                _ => return Err(format!("Unknown package manager: {}", mgr)),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "uninstall", "package": pkg, "manager": mgr_name, "output": output }).to_string())
        }

        "upgrade" => {
            let cmd = if let Some(pkg) = package {
                match mgr {
                    "brew" => format!("brew upgrade {}", pkg),
                    "apt" | "apt-get" => format!("sudo apt-get install --only-upgrade -y {}", pkg),
                    "dnf" => format!("sudo dnf upgrade -y {}", pkg),
                    "yum" => format!("sudo yum update -y {}", pkg),
                    "pacman" => format!("sudo pacman -S --noconfirm {}", pkg),
                    "zypper" => format!("sudo zypper update -y {}", pkg),
                    "apk" => format!("sudo apk upgrade {}", pkg),
                    "snap" => format!("sudo snap refresh {}", pkg),
                    "nix-env" => format!("nix-env -u {}", pkg),
                    _ => return Err(format!("Unknown package manager: {}", mgr)),
                }
            } else {
                match mgr {
                    "brew" => "brew upgrade".to_string(),
                    "apt" | "apt-get" => "sudo apt-get update && sudo apt-get upgrade -y".to_string(),
                    "dnf" => "sudo dnf upgrade -y".to_string(),
                    "yum" => "sudo yum update -y".to_string(),
                    "pacman" => "sudo pacman -Syu --noconfirm".to_string(),
                    "zypper" => "sudo zypper update -y".to_string(),
                    "apk" => "sudo apk upgrade".to_string(),
                    "snap" => "sudo snap refresh".to_string(),
                    "nix-env" => "nix-env -u".to_string(),
                    _ => return Err(format!("Unknown package manager: {}", mgr)),
                }
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "upgrade", "package": package.unwrap_or("(all)"), "manager": mgr_name, "output": output }).to_string())
        }

        "search" => {
            let query = package.ok_or("Missing 'package' (search query)")?;
            let cmd = match mgr {
                "brew" => format!("brew search {}", query),
                "apt" | "apt-get" => format!("apt-cache search {} | head -30", query),
                "dnf" => format!("dnf search {} 2>/dev/null | head -30", query),
                "yum" => format!("yum search {} 2>/dev/null | head -30", query),
                "pacman" => format!("pacman -Ss {} | head -40", query),
                "zypper" => format!("zypper search {} | head -30", query),
                "apk" => format!("apk search {} | head -30", query),
                "snap" => format!("snap find {} | head -20", query),
                "nix-env" => format!("nix-env -qaP '.*{}.*' 2>/dev/null | head -30", query),
                _ => return Err(format!("Unknown package manager: {}", mgr)),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "search", "query": query, "manager": mgr_name, "results": output }).to_string())
        }

        "list" => {
            let cmd = match mgr {
                "brew" => "brew list --versions".to_string(),
                "apt" | "apt-get" => "dpkg -l | tail -n +6 | awk '{print $2, $3}' | head -100".to_string(),
                "dnf" | "yum" => "rpm -qa --qf '%{NAME} %{VERSION}-%{RELEASE}\n' | sort | head -100".to_string(),
                "pacman" => "pacman -Q | head -100".to_string(),
                "zypper" => "zypper se --installed-only | head -100".to_string(),
                "apk" => "apk list --installed 2>/dev/null | head -100".to_string(),
                "snap" => "snap list".to_string(),
                "nix-env" => "nix-env -q | head -100".to_string(),
                _ => return Err(format!("Unknown package manager: {}", mgr)),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "list", "manager": mgr_name, "packages": output }).to_string())
        }

        "info" => {
            let pkg = package.ok_or("Missing 'package' for info action")?;
            let cmd = match mgr {
                "brew" => format!("brew info {}", pkg),
                "apt" | "apt-get" => format!("apt-cache show {} 2>/dev/null | head -40", pkg),
                "dnf" => format!("dnf info {} 2>/dev/null", pkg),
                "yum" => format!("yum info {} 2>/dev/null", pkg),
                "pacman" => format!("pacman -Si {} 2>/dev/null || pacman -Qi {} 2>/dev/null", pkg, pkg),
                "zypper" => format!("zypper info {}", pkg),
                "apk" => format!("apk info {} 2>/dev/null", pkg),
                "snap" => format!("snap info {}", pkg),
                "nix-env" => format!("nix-env -qaP --description '.*{}.*' 2>/dev/null", pkg),
                _ => return Err(format!("Unknown package manager: {}", mgr)),
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "info", "package": pkg, "manager": mgr_name, "details": output }).to_string())
        }

        "detect" => {
            Ok(json!({ "action": "detect", "manager": mgr_name, "command": mgr }).to_string())
        }

        _ => Err(format!("Unknown action: {}. Valid: install, uninstall, upgrade, search, list, info, detect", action)),
    }
}

// ── Sync implementation ─────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_pkg_manage(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: action")?;

    tracing::Span::current().record("action", action);

    let package = args.get("package").and_then(|v| v.as_str());
    let manager_override = args.get("manager").and_then(|v| v.as_str());

    let (mgr, mgr_name) = if let Some(m) = manager_override {
        (m, m)
    } else {
        let (m, n) = detect_pkg_manager();
        if m.is_empty() {
            return Err("No supported package manager found".to_string());
        }
        (m, n)
    };

    match action {
        "install" => {
            let pkg = package.ok_or("Missing 'package'")?;
            let cmd = match mgr {
                "brew" => format!("brew install {}", pkg),
                "apt" | "apt-get" => format!("sudo apt-get install -y {}", pkg),
                "dnf" => format!("sudo dnf install -y {}", pkg),
                "pacman" => format!("sudo pacman -S --noconfirm {}", pkg),
                _ => return Err(format!("Unknown manager: {}", mgr)),
            };
            let output = sh(&cmd)?;
            Ok(json!({ "action": "install", "package": pkg, "manager": mgr_name, "output": output }).to_string())
        }
        "detect" => Ok(json!({ "action": "detect", "manager": mgr_name, "command": mgr }).to_string()),
        _ => Err(format!("Sync not fully supported for '{}'. Use async.", action)),
    }
}
