//! Messenger construction from config.
//!
//! Turns a [`MessengerConfig`] into a boxed [`Messenger`], via either the
//! generic chat-system config path or a platform-specific builder.

use anyhow::{Context, Result};
use chat_system::config as chat_system_config;
use rustyclaw_core::config::MessengerConfig;
#[cfg(feature = "matrix")]
use rustyclaw_core::messengers::MatrixMessenger;
use rustyclaw_core::messengers::{GenericMessenger, Messenger};

/// Create a single messenger from config.
pub(crate) async fn create_messenger(config: &MessengerConfig) -> Result<Box<dyn Messenger>> {
    if let Some(generic_config) = generic_messenger_config(config)? {
        let mut messenger: Box<dyn Messenger> = Box::new(GenericMessenger::new(generic_config));
        messenger.initialize().await?;
        return Ok(messenger);
    }

    let name = config.name.clone();
    let mut messenger: Box<dyn Messenger> = match config.messenger_type.as_str() {
        // matrix-cli type removed - use "matrix" type instead (chat-system 0.1.3)
        "matrix-cli" => {
            anyhow::bail!("matrix-cli messenger type is deprecated. Use 'matrix' type instead.");
        }
        "irc" => build_irc_messenger(config, name)?,
        "slack" => build_slack_messenger(config, name)?,
        "google_chat" => build_google_chat_messenger(config, name)?,
        "teams" => build_teams_messenger(config, name)?,
        "imessage" => build_imessage_messenger(config, name)?,
        #[cfg(feature = "matrix")]
        "matrix" => build_matrix_messenger(config, name)?,
        #[cfg(not(feature = "matrix"))]
        "matrix" => {
            anyhow::bail!("Matrix messenger not compiled in. Rebuild with --features matrix")
        }
        other => anyhow::bail!("Unknown messenger type: {}", other),
    };

    messenger.initialize().await?;
    Ok(messenger)
}

fn generic_messenger_config(
    config: &MessengerConfig,
) -> Result<Option<chat_system_config::MessengerConfig>> {
    let name = config.name.clone();

    let messenger = match config.messenger_type.as_str() {
        "console" => Some(chat_system_config::MessengerConfig::Console(
            chat_system_config::ConsoleConfig { name },
        )),
        "discord" => Some(chat_system_config::MessengerConfig::Discord(
            chat_system_config::DiscordConfig {
                name,
                token: config.token.clone().context("Discord requires 'token'")?,
            },
        )),
        "telegram" => Some(chat_system_config::MessengerConfig::Telegram(
            chat_system_config::TelegramConfig {
                name,
                token: config.token.clone().context("Telegram requires 'token'")?,
            },
        )),
        "webhook" => Some(chat_system_config::MessengerConfig::Webhook(
            chat_system_config::WebhookConfig {
                name,
                url: config
                    .webhook_url
                    .clone()
                    .context("Webhook requires 'webhook_url'")?,
            },
        )),
        "slack" if config.app_token.is_none() && config.default_channel.is_none() => Some(
            chat_system_config::MessengerConfig::Slack(chat_system_config::SlackConfig {
                name,
                token: config.token.clone().context("Slack requires 'token'")?,
            }),
        ),
        "irc" if config.password.is_none() => Some(chat_system_config::MessengerConfig::Irc(
            chat_system_config::IrcConfig {
                name,
                server: config.server.clone().context("IRC requires 'server'")?,
                port: config.port.unwrap_or(6697),
                nick: config
                    .nick
                    .clone()
                    .unwrap_or_else(|| "RustyClaw".to_string()),
                channels: config.irc_channels.clone(),
                tls: config.use_tls.unwrap_or(false),
            },
        )),
        "google_chat"
            if config.credentials_path.is_none()
                && (config.webhook_url.is_some()
                    || (config.token.is_some() && config.spaces.len() == 1)) =>
        {
            Some(chat_system_config::MessengerConfig::GoogleChat(
                chat_system_config::GoogleChatConfig {
                    name,
                    webhook_url: config.webhook_url.clone(),
                    token: config.token.clone(),
                    space_id: config.spaces.first().cloned(),
                },
            ))
        }
        "teams" if config.app_id.is_none() && config.app_password.is_none() => Some(
            chat_system_config::MessengerConfig::Teams(chat_system_config::TeamsConfig {
                name,
                webhook_url: Some(
                    config
                        .webhook_url
                        .clone()
                        .context("Teams requires 'webhook_url'")?,
                ),
                token: None,
                team_id: None,
                channel_id: None,
            }),
        ),
        "imessage" if config.server.is_none() && config.password.is_none() => {
            Some(chat_system_config::MessengerConfig::IMessage(
                chat_system_config::IMessageConfig { name },
            ))
        }
        // Matrix is handled entirely by build_matrix_messenger to ensure
        // state_dir, allowed_chats, and dm_config are always applied.
        #[cfg(feature = "signal-cli")]
        "signal" | "signal-cli" => Some(chat_system_config::MessengerConfig::SignalCli(
            chat_system_config::SignalCliConfig {
                name,
                phone_number: config.phone.clone().context("Signal requires 'phone'")?,
                cli_path: "signal-cli".to_string(),
            },
        )),
        #[cfg(feature = "whatsapp")]
        "whatsapp" => {
            let db_path = whatsapp_state_dir(&name)?
                .join(format!("{name}.db"))
                .to_string_lossy()
                .into_owned();
            Some(chat_system_config::MessengerConfig::WhatsApp(
                chat_system_config::WhatsAppConfig { name, db_path },
            ))
        }
        _ => None,
    };

    Ok(messenger)
}

#[cfg(feature = "whatsapp")]
fn whatsapp_state_dir(name: &str) -> Result<std::path::PathBuf> {
    Ok(dirs::data_dir()
        .context("Failed to get data directory")?
        .join("rustyclaw")
        .join("whatsapp")
        .join(name))
}

fn build_irc_messenger(config: &MessengerConfig, name: String) -> Result<Box<dyn Messenger>> {
    let mut messenger = rustyclaw_core::messengers::IrcMessenger::new(
        name,
        config.server.clone().context("IRC requires 'server'")?,
        config.port.unwrap_or(6697),
        config
            .nick
            .clone()
            .unwrap_or_else(|| "RustyClaw".to_string()),
    );
    if !config.irc_channels.is_empty() {
        messenger = messenger.with_channels(config.irc_channels.clone());
    }
    if let Some(password) = config.password.clone() {
        messenger = messenger.with_password(password);
    }
    if let Some(tls) = config.use_tls {
        messenger = messenger.with_tls(tls);
    }
    Ok(Box::new(messenger))
}

fn build_slack_messenger(config: &MessengerConfig, name: String) -> Result<Box<dyn Messenger>> {
    let mut messenger = rustyclaw_core::messengers::SlackMessenger::new(
        name,
        config.token.clone().context("Slack requires 'token'")?,
    );
    if let Some(app_token) = config.app_token.clone() {
        messenger = messenger.with_app_token(app_token);
    }
    if let Some(channel) = config.default_channel.clone() {
        messenger = messenger.with_default_channel(channel);
    }
    Ok(Box::new(messenger))
}

fn build_google_chat_messenger(
    config: &MessengerConfig,
    name: String,
) -> Result<Box<dyn Messenger>> {
    if let Some(credentials_path) = config.credentials_path.clone() {
        if config.spaces.is_empty() {
            anyhow::bail!(
                "Google Chat service account mode requires at least one entry in 'spaces'"
            );
        }
        return Ok(Box::new(
            rustyclaw_core::messengers::GoogleChatMessenger::with_credentials(
                name,
                credentials_path,
                config.spaces.clone(),
            ),
        ));
    }

    if let (Some(token), Some(space_id)) = (config.token.clone(), config.spaces.first().cloned()) {
        let mut messenger =
            rustyclaw_core::messengers::GoogleChatMessenger::new_api(name, token, space_id);
        if config.spaces.len() > 1 {
            messenger = messenger.with_spaces(config.spaces[1..].to_vec());
        }
        return Ok(Box::new(messenger));
    }

    anyhow::bail!(
        "Google Chat requires 'webhook_url', 'credentials_path', or ('token' and one or more entries in 'spaces')"
    )
}

fn build_teams_messenger(config: &MessengerConfig, name: String) -> Result<Box<dyn Messenger>> {
    if let (Some(app_id), Some(app_password)) = (config.app_id.clone(), config.app_password.clone())
    {
        return Ok(Box::new(
            rustyclaw_core::messengers::TeamsMessenger::with_bot_framework(
                name,
                app_id,
                app_password,
            ),
        ));
    }

    anyhow::bail!(
        "Teams requires either 'app_id' + 'app_password' for Bot Framework mode, or 'webhook_url' for webhook mode"
    )
}

fn build_imessage_messenger(config: &MessengerConfig, name: String) -> Result<Box<dyn Messenger>> {
    if config.password.is_some() {
        anyhow::bail!(
            "BlueBubbles-backed iMessage is no longer supported; chat-system uses the local macOS Messages database"
        );
    }

    let mut messenger = rustyclaw_core::messengers::IMessageMessenger::new(name);
    if let Some(path) = config.server.clone() {
        if path.contains("://") {
            anyhow::bail!(
                "BlueBubbles-backed iMessage is no longer supported; set 'server' to a local chat.db path or omit it"
            );
        }
        messenger = messenger.with_chat_db_path(path);
    }
    Ok(Box::new(messenger))
}

#[cfg(feature = "matrix")]
fn build_matrix_messenger(config: &MessengerConfig, name: String) -> Result<Box<dyn Messenger>> {
    let homeserver = config
        .homeserver
        .clone()
        .context("Matrix requires 'homeserver'")?;
    let user_id = config
        .user_id
        .clone()
        .context("Matrix requires 'user_id'")?;

    let mut messenger = if let Some(access_token) = config.access_token.clone() {
        MatrixMessenger::with_access_token(name.clone(), homeserver, user_id, access_token, None)
    } else {
        MatrixMessenger::new(
            name.clone(),
            homeserver,
            user_id,
            config
                .password
                .clone()
                .context("Matrix requires 'password'")?,
        )
    };

    // Set state directory for sync token persistence
    if let Some(dirs) = directories::ProjectDirs::from("", "", "rustyclaw") {
        let state_dir = dirs.data_dir().join("matrix").join(&name);
        messenger = messenger.with_state_dir(state_dir);
    }

    // Set allowed chats if configured
    if !config.allowed_chats.is_empty() {
        messenger = messenger.with_allowed_chats(config.allowed_chats.clone());
    }

    // Set DM config if present
    if let Some(ref dm) = config.dm {
        use rustyclaw_core::messengers::MatrixDmConfig;
        let dm_config = MatrixDmConfig {
            enabled: dm.enabled,
            policy: dm.policy.clone().unwrap_or_else(|| "allowlist".to_string()),
            allow_from: dm.allow_from.clone(),
        };
        messenger = messenger.with_dm_config(dm_config);
    }

    Ok(Box::new(messenger))
}
