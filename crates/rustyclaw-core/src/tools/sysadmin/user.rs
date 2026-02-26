//! User and group management: list, add, remove, group membership.

use super::{sh, sh_async};
use serde_json::{Value, json};
use std::path::Path;
use tracing::{debug, instrument};

// ── Async implementation ────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub async fn exec_user_manage_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing action")?;
    tracing::Span::current().record("action", action);
    let name = args.get("name").and_then(|v| v.as_str());
    debug!(name, "User management request");

    match action {
        "whoami" => {
            let user = sh_async("whoami").await?;
            let groups = sh_async("groups 2>/dev/null").await.unwrap_or_default();
            let id_output = sh_async("id").await.unwrap_or_default();
            let sudo_check = sh_async("sudo -n true 2>&1; echo $?")
                .await
                .map(|s| s.trim() == "0")
                .unwrap_or(false);
            Ok(json!({ "action": "whoami", "user": user, "groups": groups, "id": id_output, "has_sudo": sudo_check }).to_string())
        }

        "list_users" => {
            let output = if cfg!(target_os = "macos") {
                sh_async("dscl . list /Users | grep -v '^_'").await?
            } else {
                sh_async("awk -F: '$3 >= 1000 || $3 == 0 { print $1, $3, $6, $7 }' /etc/passwd")
                    .await?
            };
            Ok(json!({ "action": "list_users", "output": output }).to_string())
        }

        "list_groups" => {
            let output = if cfg!(target_os = "macos") {
                sh_async("dscl . list /Groups | grep -v '^_' | head -40").await?
            } else {
                sh_async("awk -F: '{ print $1, $3 }' /etc/group | head -40").await?
            };
            Ok(json!({ "action": "list_groups", "output": output }).to_string())
        }

        "user_info" => {
            let user = name.ok_or("Missing 'name'")?;
            let output = if cfg!(target_os = "macos") {
                sh_async(&format!("dscl . read /Users/{} 2>&1 | head -30", user)).await?
            } else {
                sh_async(&format!(
                    "id {} 2>&1 && getent passwd {} 2>/dev/null",
                    user, user
                ))
                .await?
            };
            Ok(json!({ "action": "user_info", "user": user, "output": output }).to_string())
        }

        "add_user" => {
            let user = name.ok_or("Missing 'name'")?;
            let cmd = if cfg!(target_os = "macos") {
                format!("sudo sysadminctl -addUser {} -password '' 2>&1", user)
            } else {
                let shell = args
                    .get("shell")
                    .and_then(|v| v.as_str())
                    .unwrap_or("/bin/bash");
                format!("sudo useradd -m -s {} {} 2>&1", shell, user)
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "add_user", "user": user, "output": if output.is_empty() { format!("User '{}' created.", user) } else { output } }).to_string())
        }

        "remove_user" => {
            let user = name.ok_or("Missing 'name'")?;
            let cmd = if cfg!(target_os = "macos") {
                format!("sudo sysadminctl -deleteUser {} 2>&1", user)
            } else {
                format!("sudo userdel -r {} 2>&1", user)
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "remove_user", "user": user, "output": if output.is_empty() { format!("User '{}' removed.", user) } else { output } }).to_string())
        }

        "add_to_group" => {
            let user = name.ok_or("Missing 'name'")?;
            let group = args
                .get("group")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'group'")?;
            let cmd = if cfg!(target_os = "macos") {
                format!(
                    "sudo dseditgroup -o edit -a {} -t user {} 2>&1",
                    user, group
                )
            } else {
                format!("sudo usermod -aG {} {} 2>&1", group, user)
            };
            let output = sh_async(&cmd).await?;
            Ok(json!({ "action": "add_to_group", "user": user, "group": group, "output": if output.is_empty() { format!("User '{}' added to group '{}'.", user, group) } else { output } }).to_string())
        }

        "last_logins" => {
            let output = sh_async("last -20 2>/dev/null | head -25").await?;
            Ok(json!({ "action": "last_logins", "output": output }).to_string())
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: whoami, list_users, list_groups, user_info, add_user, remove_user, add_to_group, last_logins",
            action
        )),
    }
}

// ── Sync implementation ─────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_user_manage(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing action")?;
    tracing::Span::current().record("action", action);

    match action {
        "whoami" => {
            let user = sh("whoami")?;
            let groups = sh("groups 2>/dev/null").unwrap_or_default();
            Ok(json!({ "action": "whoami", "user": user, "groups": groups }).to_string())
        }
        "list_users" => {
            let output = if cfg!(target_os = "macos") {
                sh("dscl . list /Users | grep -v '^_'")?
            } else {
                sh("awk -F: '$3 >= 1000 || $3 == 0 { print $1 }' /etc/passwd")?
            };
            Ok(json!({ "action": "list_users", "output": output }).to_string())
        }
        _ => Err(format!(
            "Sync not fully supported for '{}'. Use async.",
            action
        )),
    }
}
