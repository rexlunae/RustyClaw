//! Pre-iocraft connection dialog for the TUI client.
//!
//! Mirrors the desktop client's `ConnectionDialog`: when no gateway URL is
//! configured (via `--url` or `Config::gateway_url`), prompt the user for a
//! URL — pre-filled with their most recently saved choice — show a spinner
//! while the SSH connection is established, and re-prompt with the error
//! message if the connection fails.

use std::io::{self, BufRead, Write};

use anyhow::Result;

use rustyclaw_core::client_prefs::{
    DEFAULT_GATEWAY_URL, load_auto_connect_gateway_urls, load_default_startup_gateway_urls,
    load_saved_gateway_url, save_gateway_url, should_bypass_connection_dialog,
};
use rustyclaw_core::gateway::{SshConnection, SshReader, SshWriter};
use rustyclaw_core::theme as t;

/// Outcome of attempting to establish a gateway connection.
pub struct ConnectionResult {
    pub connection: SshConnection,
    pub writer: SshWriter,
    pub reader: SshReader,
    pub url: String,
}

/// Prompt for a gateway URL (when needed) and establish the SSH connection.
///
/// * If `explicit_url` is `Some`, skip the dialog and try to connect once;
///   on success the URL is saved for next time, on failure the error is
///   returned to the caller (which is expected to surface it the same way
///   as before this dialog existed).
/// * If `skip_dialog` is `true`, behave like `explicit_url` was provided
///   but fall back to the saved URL or the built-in default when none
///   was supplied on the command line.
/// * Otherwise, walk the user through an interactive prompt loop with the
///   previously-saved URL pre-filled. The user may accept the default,
///   edit it, or cancel.
pub async fn prompt_and_connect(
    explicit_url: Option<String>,
    skip_dialog: bool,
) -> Result<Option<ConnectionResult>> {
    let mut direct_urls = if let Some(url) = explicit_url.clone() {
        vec![url]
    } else {
        Vec::new()
    };
    let bypass_dialog = skip_dialog || should_bypass_connection_dialog();
    if direct_urls.is_empty() && bypass_dialog {
        direct_urls = load_auto_connect_gateway_urls();
        if direct_urls.is_empty() && skip_dialog {
            direct_urls = load_default_startup_gateway_urls();
        }
        if direct_urls.is_empty() && skip_dialog {
            direct_urls.push(load_saved_gateway_url().unwrap_or_else(|| DEFAULT_GATEWAY_URL.to_string()));
        }
    }

    if !direct_urls.is_empty() {
        let mut last_error: Option<String> = None;
        for url in direct_urls {
            let pb = t::spinner(&format!("Connecting to {}…", url));
            match SshConnection::connect(&url).await {
                Ok((connection, writer, reader)) => {
                    t::spinner_ok(&pb, &format!("Connected to {}", url));
                    save_gateway_url(&url);
                    return Ok(Some(ConnectionResult {
                        connection,
                        writer,
                        reader,
                        url,
                    }));
                }
                Err(e) => {
                    t::spinner_fail(&pb, &format!("SSH connection failed: {}", e));
                    last_error = Some(e.to_string());
                }
            }
        }

        if let Some(err) = last_error {
            // Mirror the previous behaviour: bubble the error path up so
            // the gateway-event channel reports it.
            return Err(anyhow::anyhow!("SSH connection failed: {}", err));
        }
    }

    // Interactive prompt loop.
    let default_url = load_default_startup_gateway_urls()
        .into_iter()
        .next()
        .or_else(load_saved_gateway_url)
        .unwrap_or_else(|| DEFAULT_GATEWAY_URL.to_string());
    let mut current = default_url;
    let stdin = io::stdin();
    let mut reader = stdin.lock();

    println!();
    t::print_header("🦀  Connect to gateway  🦀");
    println!();
    println!(
        "  {}",
        t::muted("RustyClaw connects to your gateway over SSH (default port 2222).")
    );
    println!(
        "  {}",
        t::muted("Press Enter to accept the default, or type a new URL.")
    );
    println!(
        "  {}",
        t::muted("Type 'cancel' (or press Ctrl+C) to abort.")
    );
    println!();

    loop {
        let prompt = format!("{} ", t::accent(&format!("Gateway URL [{}]:", current)));
        print!("{}", prompt);
        io::stdout().flush()?;
        let mut buf = String::new();
        if reader.read_line(&mut buf)? == 0 {
            // EOF — treat as cancel.
            return Ok(None);
        }
        let entered = buf
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .trim()
            .to_string();

        if entered.eq_ignore_ascii_case("cancel") || entered.eq_ignore_ascii_case("quit") {
            return Ok(None);
        }
        if !entered.is_empty() {
            current = entered;
        }

        let pb = t::spinner(&format!("Connecting to {}…", current));
        match SshConnection::connect(&current).await {
            Ok((connection, writer, reader)) => {
                t::spinner_ok(&pb, &format!("Connected to {}", current));
                save_gateway_url(&current);
                return Ok(Some(ConnectionResult {
                    connection,
                    writer,
                    reader,
                    url: current,
                }));
            }
            Err(e) => {
                t::spinner_fail(&pb, &format!("SSH connection failed: {}", e));
                println!(
                    "  {}",
                    t::muted("Edit the URL and try again, or type 'cancel' to abort.")
                );
            }
        }
    }
}
