use anyhow::Result;
use tracing::{debug, instrument, warn};

use rustyclaw_core::gateway::protocol::server::{
    send_secrets_delete_credential_result, send_secrets_delete_result, send_secrets_get_result,
    send_secrets_has_totp_result, send_secrets_list_result, send_secrets_peek_result,
    send_secrets_remove_totp_result, send_secrets_set_disabled_result,
    send_secrets_set_policy_result, send_secrets_setup_totp_result, send_secrets_store_result,
    send_secrets_verify_totp_result, send_vault_unlocked,
};
use rustyclaw_core::gateway::{ClientPayload, SecretEntryDto, transport};
use rustyclaw_core::secrets::{
    AccessContext, AccessPolicy, CredentialValue, SecretEntry, SecretKind,
};

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
pub fn format_credential_value(name: &str, entry: &SecretEntry, value: &CredentialValue) -> String {
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
        CredentialValue::SshKeyPair {
            private_key,
            public_key,
        } => {
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

    debug!(
        credential = cred_name,
        kind = kind_str,
        "Storing credential"
    );

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
        return Err("username_password credentials require the 'username' parameter.".into());
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

    debug!(
        credential = cred_name,
        policy = policy_str,
        "Setting credential policy"
    );

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
    Ok(format!("Policy for '{}' set to '{}'.", cred_name, policy,))
}

/// Handle a vault/secrets client frame against the shared vault.
///
/// Covers the `UnlockVault` and `Secrets*` protocol payloads sent by a
/// connected client. Each arm locks the vault, performs the requested
/// operation, and streams a typed result frame back over `writer`. Payloads
/// outside the secrets family are ignored (the caller is expected to route
/// only secrets variants here).
pub(crate) async fn handle_secrets_frame(
    writer: &mut dyn transport::TransportWriter,
    vault: &SharedVault,
    payload: ClientPayload,
) -> Result<()> {
    match payload {
        ClientPayload::UnlockVault { password } => {
            let mut v = vault.lock().await;
            v.set_password(password);
            match v.get_secret("__vault_check__", true) {
                Ok(_) => {
                    send_vault_unlocked(writer, true, None).await?;
                }
                Err(e) => {
                    v.clear_password();
                    send_vault_unlocked(
                        writer,
                        false,
                        Some(&format!("Failed to unlock vault: {}", e)),
                    )
                    .await?;
                }
            }
        }
        ClientPayload::SecretsList => {
            let mut v = vault.lock().await;
            let entries = v.list_all_entries();
            let dto_entries: Vec<SecretEntryDto> = entries
                .iter()
                .map(|(name, entry)| SecretEntryDto {
                    name: name.clone(),
                    label: entry.label.clone(),
                    kind: entry.kind.to_string(),
                    policy: entry.policy.badge().to_string(),
                    disabled: entry.disabled,
                })
                .collect();
            send_secrets_list_result(writer, true, dto_entries).await?;
        }
        ClientPayload::SecretsStore { key, value } => {
            let mut v = vault.lock().await;
            let result = v.store_secret(&key, &value);
            match result {
                Ok(()) => {
                    send_secrets_store_result(writer, true, &format!("Secret '{}' stored.", key))
                        .await?
                }
                Err(e) => {
                    send_secrets_store_result(
                        writer,
                        false,
                        &format!("Failed to store secret: {}", e),
                    )
                    .await?
                }
            };
        }
        ClientPayload::SecretsGet { key } => {
            let mut v = vault.lock().await;
            let result = v.get_secret(&key, true);
            match result {
                Ok(Some(value)) => {
                    send_secrets_get_result(writer, true, &key, Some(&value), None).await?
                }
                Ok(None) => {
                    send_secrets_get_result(
                        writer,
                        false,
                        &key,
                        None,
                        Some(&format!("Secret '{}' not found.", key)),
                    )
                    .await?
                }
                Err(e) => {
                    send_secrets_get_result(
                        writer,
                        false,
                        &key,
                        None,
                        Some(&format!("Failed to get secret: {}", e)),
                    )
                    .await?
                }
            };
        }
        ClientPayload::SecretsDelete { key } => {
            let mut v = vault.lock().await;
            let result = v.delete_secret(&key);
            match result {
                Ok(()) => send_secrets_delete_result(writer, true, None).await?,
                Err(e) => {
                    send_secrets_delete_result(
                        writer,
                        false,
                        Some(&format!("Failed to delete: {}", e)),
                    )
                    .await?
                }
            };
        }
        ClientPayload::SecretsPeek { name } => {
            let mut v = vault.lock().await;
            let result = v.peek_credential_display(&name);
            match result {
                Ok(fields) => {
                    let field_tuples: Vec<(String, String)> = fields
                        .iter()
                        .map(|(label, value)| (label.clone(), value.clone()))
                        .collect();
                    send_secrets_peek_result(writer, true, field_tuples, None).await?
                }
                Err(e) => {
                    send_secrets_peek_result(
                        writer,
                        false,
                        vec![],
                        Some(&format!("Failed to peek: {}", e)),
                    )
                    .await?
                }
            };
        }
        ClientPayload::SecretsSetPolicy {
            name,
            policy,
            skills,
        } => {
            let mut v = vault.lock().await;
            let policy_str = policy.clone();
            let policy = match policy.as_str() {
                "always" => Some(AccessPolicy::Always),
                "ask" => Some(AccessPolicy::WithApproval),
                "auth" => Some(AccessPolicy::WithAuth),
                "skill_only" => Some(AccessPolicy::SkillOnly(skills)),
                _ => None,
            };
            if let Some(policy) = policy {
                let result = v.set_credential_policy(&name, policy);
                match result {
                    Ok(()) => send_secrets_set_policy_result(writer, true, None).await?,
                    Err(e) => {
                        send_secrets_set_policy_result(
                            writer,
                            false,
                            Some(&format!("Failed to set policy: {}", e)),
                        )
                        .await?
                    }
                }
            } else {
                send_secrets_set_policy_result(
                    writer,
                    false,
                    Some(&format!("Unknown policy: {}", policy_str)),
                )
                .await?;
            }
        }
        ClientPayload::SecretsSetDisabled { name, disabled } => {
            let mut v = vault.lock().await;
            let result = v.set_credential_disabled(&name, disabled);
            match result {
                Ok(()) => send_secrets_set_disabled_result(writer, true, None).await?,
                Err(e) => {
                    send_secrets_set_disabled_result(writer, false, Some(&format!("Failed: {}", e)))
                        .await?
                }
            };
        }
        ClientPayload::SecretsDeleteCredential { name } => {
            let mut v = vault.lock().await;
            let meta_key = format!("cred:{}", name);
            let is_legacy = v.get_secret(&meta_key, true).ok().flatten().is_none();
            if is_legacy {
                let _ = v.delete_secret(&name);
            }
            let result = v.delete_credential(&name);
            match result {
                Ok(()) => send_secrets_delete_credential_result(writer, true, None).await?,
                Err(e) => {
                    send_secrets_delete_credential_result(
                        writer,
                        false,
                        Some(&format!("Failed: {}", e)),
                    )
                    .await?
                }
            };
        }
        ClientPayload::SecretsHasTotp => {
            let mut v = vault.lock().await;
            let has_totp = v.has_totp();
            send_secrets_has_totp_result(writer, has_totp).await?;
        }
        ClientPayload::SecretsSetupTotp => {
            let mut v = vault.lock().await;
            let result = v.setup_totp("rustyclaw");
            match result {
                Ok(uri) => send_secrets_setup_totp_result(writer, true, Some(&uri), None).await?,
                Err(e) => {
                    send_secrets_setup_totp_result(
                        writer,
                        false,
                        None,
                        Some(&format!("Failed: {}", e)),
                    )
                    .await?
                }
            };
        }
        ClientPayload::SecretsVerifyTotp { code } => {
            let mut v = vault.lock().await;
            let result = v.verify_totp(&code);
            match result {
                Ok(valid) => send_secrets_verify_totp_result(writer, valid, None).await?,
                Err(e) => {
                    send_secrets_verify_totp_result(writer, false, Some(&format!("Error: {}", e)))
                        .await?
                }
            };
        }
        ClientPayload::SecretsRemoveTotp => {
            let mut v = vault.lock().await;
            let result = v.remove_totp();
            match result {
                Ok(()) => send_secrets_remove_totp_result(writer, true, None).await?,
                Err(e) => {
                    send_secrets_remove_totp_result(writer, false, Some(&format!("Failed: {}", e)))
                        .await?
                }
            };
        }
        _ => {}
    }
    Ok(())
}
