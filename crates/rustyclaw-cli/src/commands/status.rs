//! `rustyclaw status` — show gateway, model, and workspace status.

use clap::Args;
use rustyclaw_core::config::Config;

/// Arguments for `rustyclaw status`.
#[derive(Debug, Args, Default)]
pub struct StatusArgs {
    /// Output JSON
    #[arg(long)]
    pub json: bool,
    /// Show all available information
    #[arg(long)]
    pub all: bool,
    /// Include usage statistics
    #[arg(long)]
    pub usage: bool,
    /// Verbose output
    #[arg(long, short)]
    pub verbose: bool,
}

/// Print system status to stdout.
pub(crate) fn run(config: &Config, args: &StatusArgs) {
    if args.json {
        // Minimal JSON blob — extend as features land.
        println!("{{");
        println!("  \"settings_dir\": \"{}\",", config.settings_dir.display());
        println!(
            "  \"workspace_dir\": \"{}\",",
            config.workspace_dir().display()
        );
        if let Some(m) = &config.model {
            println!("  \"provider\": \"{}\",", m.provider);
            if let Some(model) = &m.model {
                println!("  \"model\": \"{}\",", model);
            }
        }
        if let Some(gw) = &config.gateway_url {
            println!("  \"gateway_url\": \"{}\"", gw);
        }
        println!("}}");
    } else {
        use rustyclaw_core::theme as t;
        println!("{}\n", t::heading("RustyClaw status"));
        println!(
            "{}",
            t::label_value("Settings dir", &config.settings_dir.display().to_string())
        );
        println!(
            "{}",
            t::label_value(
                "Workspace   ",
                &config.workspace_dir().display().to_string()
            )
        );
        if let Some(m) = &config.model {
            println!("{}", t::label_value("Provider    ", &m.provider));
            if let Some(model) = &m.model {
                println!("{}", t::label_value("Model       ", model));
            }
        } else {
            println!(
                "  {} : {}",
                t::muted("Provider    "),
                t::warn(&format!(
                    "(not configured — run {})",
                    t::accent_bright("`rustyclaw onboard`")
                ))
            );
        }
        if let Some(gw) = &config.gateway_url {
            println!("{}", t::label_value("Gateway URL ", gw));
        }
        if args.verbose || args.all {
            println!(
                "{}",
                t::label_value("SOUL.md     ", &config.soul_path().display().to_string())
            );
            println!(
                "{}",
                t::label_value("Skills dir  ", &config.skills_dir().display().to_string())
            );
            println!(
                "{}",
                t::label_value(
                    "Credentials ",
                    &config.credentials_dir().display().to_string()
                )
            );
        }
    }
}
