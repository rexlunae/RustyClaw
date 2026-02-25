use tracing::{debug, warn, instrument};

use crate::secrets::{AccessContext, AccessPolicy, CredentialValue, SecretEntry, SecretKind};

use super::SharedVault;

/// Execute a secrets-vault tool against the shared vault.
///
/// These are intercepted before the generic `tools::execute_tool` path
/// because they require `SharedVault` access — the normal tool signature
/// only receives `(args, workspace_dir)`.
///
/// Access control is delegated entirely to [`SecretsManager::check_access`]
/// and the per-credential [`AccessPolicy`].  The agent gets an
/// [`AccessContext`] with `user_approved = false` (the tool invocation
/// itself does not constitute user approval) and `authenticated = false`
/// (no re-auth has occurred).  This means:
///
/// - `Always` credentials are readable.
/// - `WithApproval` credentials are only readable if `agent_access_enabled`
///   is set in config.
/// - `WithAuth` and `SkillOnly` credentials are denied.
#[instrument(skip(args, vault), fields(%name))]
pub async fn execute_secrets_tool(
    name: &str,
    args: &serde_json::Value,
    vault: &SharedVault,
) -> Result<String, String> {
    debug!("Executing secrets tool");
    match name {
        "secrets_list" => exec_secrets_list(vault).await,
        "secrets_get" => exec_secrets_get(args, vault).await,
        "secrets_store" => exec_secrets_store(args, vault).await,
        "secrets_set_policy" => exec_secrets_set_policy(args, vault).await,
        _ => {
            warn!("Unknown secrets tool requested");
            Err(format!("Unknown secrets tool: {}", name))
        }
    }
}

/// List all credentials in the vault (names, kinds, policies — no values).
#[instrument(skip(vault))]
pub async fn exec_secrets_list(vault: &SharedVault) -> Result<String, String> {
    let mut mgr = vault.lock().await;
    let entries = mgr.list_all_entries();

    if entries.is_empty() {
        debug!("Vault is empty");
        return Ok("No credentials stored in the vault.".into());
    }

    debug!(count = entries.len(), "Listing vault credentials");
    let mut lines = Vec::with_capacity(entries.len() + 1);
    lines.push(format!("{} credential(s) in vault:\n", entries.len()));

    for (name, entry) in &entries {
        let disabled = if entry.disabled { " [DISABLED]" } else { "" };
        let desc = entry
            .description
            .as_deref()
            .map(|d| format!(" — {}", d))
            .unwrap_or_default();
        lines.push(format!(
            "  • {} ({}, policy: {}){}{}\n",
            name, entry.kind, entry.policy, disabled, desc,
        ));
    }

    Ok(lines.join(""))
}

/// Retrieve a single credential value from the vault.
#[instrument(skip(args, vault))]
pub async fn exec_secrets_get(
    args: &serde_json::Value,
    vault: &SharedVault,
) -> Result<String, String> {
    let cred_name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    debug!(credential = cred_name, "Retrieving credential");

    let ctx = AccessContext {
        user_approved: false,
        authenticated: false,
        active_skill: None,
    };

    let mut mgr = vault.lock().await;
    match mgr.get_credential(cred_name, &ctx) {
        Ok(Some((entry, value))) => {
            debug!(credential = cred_name, "Credential retrieved successfully");
            Ok(format_credential_value(cred_name, &entry, &value))
        }
        Ok(None) => {
            debug!(credential = cred_name, "Credential not found");
            Err(format!(
                "Credential '{}' not found. Use secrets_list to see available credentials.",
                cred_name,
            ))
        }
        Err(e) => {
            warn!(credential = cred_name, error = %e, "Credential access denied");
            Err(e.to_string())
        }
    }
}

/// Format a credential value for returning to the model.
pub fn format_credential_value(
    name: &str,
    entry: &SecretEntry,
    value: &CredentialValue,
) -> String {
    match value {
        CredentialValue::Single(v) => {
            format!("[{}] {} = {}", entry.kind, name, v)
        }
        CredentialValue::UserPass { username, password } => {
            format!(
                "[{}] {}\n  username: {}\n  password: {}",
                entry.kind, name, username, password,
            )
        }
        CredentialValue::SshKeyPair { private_key, public_key } => {
            format!(
                "[{}] {}\n  public_key: {}\n  private_key: <{} chars>",
                entry.kind,
                name,
                public_key,
                private_key.len(),
            )
        }
        CredentialValue::FormFields(fields) => {
            let mut out = format!("[{}] {}\n", entry.kind, name);
            for (k, v) in fields {
                out.push_str(&format!("  {}: {}\n", k, v));
            }
            out
        }
        CredentialValue::PaymentCard {
            cardholder,
            number,
            expiry,
            cvv,
            extra,
        } => {
            let mut out = format!(
                "[{}] {}\n  cardholder: {}\n  number: {}\n  expiry: {}\n  cvv: {}",
                entry.kind, name, cardholder, number, expiry, cvv,
            );
            for (k, v) in extra {
                out.push_str(&format!("\n  {}: {}", k, v));
            }
            out
        }
    }
}

/// Store a new credential in the vault.
#[instrument(skip(args, vault))]
pub async fn exec_secrets_store(
    args: &serde_json::Value,
    vault: &SharedVault,
) -> Result<String, String> {
    let cred_name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    let kind_str = args
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: kind".to_string())?;

    let value = args
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: value".to_string())?;

    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let username = args.get("username").and_then(|v| v.as_str());

    let policy_str = args.get("policy").and_then(|v| v.as_str());

    debug!(credential = cred_name, kind = kind_str, "Storing credential");

    let kind = match kind_str {
        "api_key" => SecretKind::ApiKey,
        "token" => SecretKind::Token,
        "username_password" => SecretKind::UsernamePassword,
        "ssh_key" => SecretKind::SshKey,
        "secure_note" => SecretKind::SecureNote,
        "http_passkey" => SecretKind::HttpPasskey,
        "form_autofill" => SecretKind::FormAutofill,
        "payment_method" => SecretKind::PaymentMethod,
        "other" => SecretKind::Other,
        _ => {
            warn!(kind = kind_str, "Unknown credential kind");
            return Err(format!(
                "Unknown credential kind: '{}'. Use one of: api_key, token, \
                 username_password, ssh_key, secure_note, http_passkey, \
                 form_autofill, payment_method, other.",
                kind_str,
            ));
        }
    };

    let policy = match policy_str {
        Some("always") | Some("open") => AccessPolicy::Always,
        Some("approval") | Some("ask") | None => AccessPolicy::WithApproval,
        Some("auth") => AccessPolicy::WithAuth,
        Some(s) if s.starts_with("skill:") => {
            let skill_name = s.strip_prefix("skill:").unwrap();
            AccessPolicy::SkillOnly(vec![skill_name.to_string()])
        }
        Some(s) => {
            warn!(policy = s, "Unknown policy");
            return Err(format!(
                "Unknown policy: '{}'. Use: always, approval, auth, or skill:<name>.",
                s,
            ));
        }
    };

    if kind == SecretKind::UsernamePassword && username.is_none() {
        return Err(
            "username_password credentials require the 'username' parameter.".into(),
        );
    }

    let entry = SecretEntry {
        label: cred_name.to_string(),
        kind,
        policy: policy.clone(),
        description,
        disabled: false,
    };

    let mut mgr = vault.lock().await;
    mgr.store_credential(cred_name, &entry, value, username)
        .map_err(|e| {
            warn!(credential = cred_name, error = %e, "Failed to store credential");
            format!("Failed to store credential: {}", e)
        })?;

    debug!(credential = cred_name, "Credential stored successfully");
    Ok(format!(
        "Credential '{}' stored successfully (kind: {}, policy: {}).",
        cred_name, entry.kind, entry.policy,
    ))
}

/// Change the access policy of an existing credential.
#[instrument(skip(args, vault))]
pub async fn exec_secrets_set_policy(
    args: &serde_json::Value,
    vault: &SharedVault,
) -> Result<String, String> {
    let cred_name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: name".to_string())?;

    let policy_str = args
        .get("policy")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: policy".to_string())?;

    debug!(credential = cred_name, policy = policy_str, "Setting credential policy");

    let policy = match policy_str {
        "always" | "open" => AccessPolicy::Always,
        "approval" | "ask" => AccessPolicy::WithApproval,
        "auth" => AccessPolicy::WithAuth,
        s if s.starts_with("skill:") => {
            let skill_name = s.strip_prefix("skill:").unwrap();
            AccessPolicy::SkillOnly(vec![skill_name.to_string()])
        }
        _ => {
            warn!(policy = policy_str, "Unknown policy");
            return Err(format!(
                "Unknown policy: '{}'. Use: always, approval, auth, or skill:<name>.",
                policy_str,
            ));
        }
    };

    let mut mgr = vault.lock().await;
    mgr.set_credential_policy(cred_name, policy.clone())
        .map_err(|e| {
            warn!(credential = cred_name, error = %e, "Failed to set policy");
            format!("Failed to set policy: {}", e)
        })?;

    debug!(credential = cred_name, "Policy updated successfully");
    Ok(format!(
        "Policy for '{}' set to '{}'.",
        cred_name, policy,
    ))
}
