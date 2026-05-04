//! Gateway command handlers.
//!
//! Extracted from main.rs for modularity. Handles `rustyclaw gateway <sub>` commands.

use anyhow::Result;
use rustyclaw_core::config::Config;
use rustyclaw_core::daemon;
use rustyclaw_core::gateway::{GatewayOptions, ModelContext};
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::skills::SkillManager;
use rustyclaw_core::theme as t;
use tokio_util::sync::CancellationToken;

/// Handle `gateway start` command.
pub fn handle_start(
    config: &Config,
    vault_password: Option<&str>,
    ssh_listen: &str,
) -> Result<()> {
    let sp = t::spinner("Starting gateway…");

    match daemon::start(
        &config.settings_dir,
        ssh_listen,
        &[],
        None,
        vault_password,
        config.tls_cert.as_deref(),
        config.tls_key.as_deref(),
    ) {
        Ok(pid) => {
            t::spinner_ok(
                &sp,
                &format!(
                    "Gateway started (PID {}, SSH {})",
                    pid,
                    t::info(ssh_listen),
                ),
            );
            println!(
                "  {}",
                t::muted(&format!(
                    "Logs: {}",
                    daemon::log_path(&config.settings_dir).display()
                ))
            );
        }
        Err(e) => {
            t::spinner_fail(&sp, &format!("Failed to start gateway: {}", e));
        }
    }
    Ok(())
}

/// Handle `gateway stop` command.
pub fn handle_stop(config: &Config) -> Result<()> {
    let sp = t::spinner("Stopping gateway…");

    match daemon::stop(&config.settings_dir)? {
        daemon::StopResult::Stopped { pid } => {
            t::spinner_ok(&sp, &format!("Gateway stopped (was PID {})", pid));
        }
        daemon::StopResult::WasStale { pid } => {
            t::spinner_warn(
                &sp,
                &format!("Cleaned up stale PID file (PID {} was not running)", pid),
            );
        }
        daemon::StopResult::WasNotRunning => {
            t::spinner_warn(&sp, "Gateway is not running");
        }
    }
    Ok(())
}

/// Handle `gateway restart` command.
pub fn handle_restart(
    config: &Config,
    vault_password: Option<&str>,
    ssh_listen: &str,
) -> Result<()> {
    let sp = t::spinner("Restarting gateway…");

    // Stop first (ignore "not running" errors).
    let was_running = match daemon::stop(&config.settings_dir) {
        Ok(daemon::StopResult::Stopped { pid }) => {
            sp.set_message(format!("Stopped PID {}. Starting…", pid));
            true
        }
        Ok(_) => false,
        Err(e) => {
            t::spinner_fail(&sp, &format!("Failed to stop: {}", e));
            return Ok(());
        }
    };

    // Brief pause to let the port free up.
    if was_running {
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    match daemon::start(
        &config.settings_dir,
        ssh_listen,
        &[],
        None,
        vault_password,
        config.tls_cert.as_deref(),
        config.tls_key.as_deref(),
    ) {
        Ok(pid) => {
            t::spinner_ok(
                &sp,
                &format!(
                    "Gateway restarted (PID {}, SSH {})",
                    pid,
                    t::info(ssh_listen),
                ),
            );
        }
        Err(e) => {
            t::spinner_fail(&sp, &format!("Failed to start: {}", e));
        }
    }
    Ok(())
}

/// Handle `gateway status` command.
pub fn handle_status(config: &Config, json: bool) {
    let ssh_addr = config
        .ssh
        .as_ref()
        .map(|s| s.bind.clone())
        .unwrap_or_else(|| "0.0.0.0:2222".to_string());
    let status = daemon::status(&config.settings_dir);

    if json {
        let (running, pid) = match &status {
            daemon::DaemonStatus::Running { pid } => (true, Some(*pid)),
            daemon::DaemonStatus::Stale { pid } => (false, Some(*pid)),
            daemon::DaemonStatus::Stopped => (false, None),
        };
        print!("{{ \"running\": {}", running);
        if let Some(pid) = pid {
            print!(", \"pid\": {}", pid);
        }
        println!(", \"ssh_listen\": \"{}\" }}", ssh_addr);
    } else {
        println!("{}", t::label_value("SSH Listen  ", &ssh_addr));
        match status {
            daemon::DaemonStatus::Running { pid } => {
                println!(
                    "{}",
                    t::label_value(
                        "Status     ",
                        &t::success(&format!("running (PID {})", pid))
                    )
                );
            }
            daemon::DaemonStatus::Stale { pid } => {
                println!(
                    "{}",
                    t::label_value(
                        "Status     ",
                        &t::warn(&format!("stale PID file (PID {} not running)", pid))
                    )
                );
            }
            daemon::DaemonStatus::Stopped => {
                println!("{}", t::label_value("Status     ", &t::muted("stopped")));
            }
        }
        let log = daemon::log_path(&config.settings_dir);
        if log.exists() {
            println!(
                "{}",
                t::label_value("Log        ", &log.display().to_string())
            );
        }
    }
}

/// Handle `gateway reload` command.
#[allow(dead_code)]
pub fn handle_reload_result(result: Result<(String, String), String>) {
    let sp = t::spinner("Reloading gateway configuration…");

    match result {
        Ok((provider, model)) => {
            t::spinner_ok(
                &sp,
                &format!(
                    "Gateway reloaded: {} / {}",
                    t::info(&provider),
                    t::info(&model)
                ),
            );
        }
        Err(e) => {
            t::spinner_fail(&sp, &format!("Reload failed: {}", e));
        }
    }
}

/// Handle `gateway run` command (foreground mode).
pub async fn handle_run(config: Config, host: &str, port: u16) -> Result<()> {
    use rustyclaw_core::gateway::run_gateway;

    let listen = format!("{}:{}", host, port);
    let tls_cert = config.tls_cert.clone();
    let tls_key = config.tls_key.clone();
    let scheme = if tls_cert.is_some() { "wss" } else { "ws" };

    println!(
        "{}",
        t::icon_ok(&format!(
            "RustyClaw gateway listening on {}",
            t::info(&format!("{}://{}", scheme, listen))
        ))
    );

    // Open the secrets vault — the gateway owns it.
    let creds_dir = config.credentials_dir();
    let vault = if config.secrets_password_protected {
        let password = rpassword::prompt_password(format!("{} Vault password: ", t::info("🔑")))
            .unwrap_or_default();
        SecretsManager::with_password(&creds_dir, password)
    } else {
        SecretsManager::new(&creds_dir)
    };

    let shared_vault: rustyclaw_core::gateway::SharedVault =
        std::sync::Arc::new(tokio::sync::Mutex::new(vault));

    // Resolve model context from the vault.
    let model_ctx = {
        let mut v = shared_vault.lock().await;
        match ModelContext::resolve(&config, &mut v) {
            Ok(ctx) => {
                println!(
                    "{} {} via {} ({})",
                    t::icon_ok("Model:"),
                    t::info(&ctx.model),
                    t::info(&ctx.provider),
                    t::muted(&ctx.base_url),
                );
                Some(ctx)
            }
            Err(err) => {
                eprintln!("⚠ Could not resolve model context: {}", err);
                None
            }
        }
    };

    let cancel = CancellationToken::new();

    // Load skills for the gateway from multiple directories.
    let skills_dirs = config.skills_dirs();

    let mut sm = SkillManager::with_dirs(skills_dirs);
    if let Err(e) = sm.load_skills() {
        eprintln!("⚠ Could not load skills: {}", e);
    }
    if let Some(url) = config.clawhub_url.as_deref() {
        sm.set_registry(url, config.clawhub_token.clone());
    }
    let shared_skills: rustyclaw_core::gateway::SharedSkillManager =
        std::sync::Arc::new(tokio::sync::Mutex::new(sm));

    run_gateway(
        config,
        GatewayOptions {
            listen,
            tls_cert,
            tls_key,
            ..Default::default()
        },
        model_ctx,
        shared_vault,
        shared_skills,
        None,
        None,
        None, // observer
        cancel,
    )
    .await?;

    Ok(())
}
