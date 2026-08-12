//! Component data for the gateway-control panel.
//!
//! Two things share this panel and they are deliberately kept apart:
//!
//! - The **local daemon** — the `rustyclaw-gateway` process this machine
//!   started, tracked by the PID file under the settings directory. It can be
//!   started, stopped and restarted whether or not a client is connected to
//!   it, because managing it is a local-process operation.
//!
//! - The **connected gateway** — whatever the client is currently talking to,
//!   local or remote. The only lifecycle command the wire protocol carries is
//!   `reload`, and it needs no input from anyone, so it is the one control
//!   that works against a remote host.
//!
//! Conflating the two would produce the panel's worst possible bug: a "Stop"
//! button that looks like it acts on the gateway named at the top of the
//! dialog while actually killing a different process on this machine. The
//! data type keeps them as separate fields and the panel labels them
//! separately.

/// Whether the local gateway daemon is running, per its PID file.
///
/// Mirrors [`rustyclaw_core::daemon::DaemonStatus`] rather than re-using it so
/// this crate stays a plain data layer — and so `Stale` keeps its meaning
/// here: a PID file naming a process that is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalDaemonState {
    /// A live process with this PID.
    Running { pid: u32 },
    /// The PID file names a process that is no longer alive.
    Stale { pid: u32 },
    /// No PID file — nothing was started, or it was stopped cleanly.
    Stopped,
}

impl LocalDaemonState {
    /// Build from the core daemon status.
    pub fn from_status(status: &rustyclaw_core::daemon::DaemonStatus) -> Self {
        match status {
            rustyclaw_core::daemon::DaemonStatus::Running { pid } => Self::Running { pid: *pid },
            rustyclaw_core::daemon::DaemonStatus::Stale { pid } => Self::Stale { pid: *pid },
            rustyclaw_core::daemon::DaemonStatus::Stopped => Self::Stopped,
        }
    }

    /// Read the daemon status for a settings directory.
    pub fn probe(settings_dir: &std::path::Path) -> Self {
        Self::from_status(&rustyclaw_core::daemon::status(settings_dir))
    }

    /// Short status word for the badge.
    pub fn label(&self) -> String {
        match self {
            Self::Running { pid } => format!("running (PID {pid})"),
            Self::Stale { pid } => format!("stale PID file (PID {pid} is gone)"),
            Self::Stopped => "stopped".to_string(),
        }
    }

    /// Bulma-style modifier for the badge.
    pub fn status_class(&self) -> &'static str {
        match self {
            Self::Running { .. } => "is-success",
            Self::Stale { .. } => "is-warning",
            Self::Stopped => "is-light",
        }
    }

    /// The recorded PID, whether or not the process is still alive.
    pub fn pid(&self) -> Option<u32> {
        match self {
            Self::Running { pid } | Self::Stale { pid } => Some(*pid),
            Self::Stopped => None,
        }
    }

    /// Whether a live gateway process is behind this state.
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

/// Whether a gateway URL names this machine.
///
/// Only used to tell the user whether the daemon controls below apply to the
/// gateway they are connected to, so the cost of the two possible mistakes is
/// asymmetric: calling a remote gateway "this machine" is misleading, while
/// calling a loopback gateway "remote" merely under-claims. The check is
/// therefore conservative — the literal loopback forms and nothing more. A
/// LAN address that happens to be this host's own reads as remote, which is
/// the safe direction to be wrong in.
///
/// A URL that does not parse also reads as remote: an unparseable URL is one
/// the client could not have connected through either.
pub fn url_is_loopback(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    match parsed.host() {
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        Some(url::Host::Domain(name)) => {
            // `ssh` is not one of the URL standard's "special" schemes, so the
            // parser leaves the host opaque and hands back `Host::Domain`
            // even for a dotted quad — `Host::Ipv4` only ever appears for
            // `http`/`ws`/etc. Every gateway URL in the tree is `ssh://`, so
            // the IPv4 arm above would otherwise be the arm that never runs
            // and this one would call `127.0.0.1` a remote host.
            if let Ok(addr) = name.parse::<std::net::IpAddr>() {
                return addr.is_loopback();
            }
            // `localhost` is the only name guaranteed to resolve to loopback
            // (RFC 6761 §6.3).
            name.eq_ignore_ascii_case("localhost")
        }
        None => false,
    }
}

/// Everything the gateway-control panel renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayControlData {
    /// The gateway URL the client is configured to talk to.
    pub url: String,
    /// Whether a client session is currently established.
    pub connected: bool,
    /// State of the local daemon, re-probed whenever the panel opens or an
    /// action finishes.
    pub local: LocalDaemonState,
    /// Address the local daemon would listen on, from the local config.
    pub ssh_listen: String,
    /// Path to the local daemon's log file, shown so the user can go read it
    /// when a start fails — the failure itself lands in that file, not here.
    pub log_path: String,
    /// Whether the local vault is password-protected. When it is, a daemon
    /// started from here comes up with the vault locked, because this panel
    /// deliberately never asks for the password.
    pub vault_password_protected: bool,
    /// The action in flight, if any. Every button is disabled until it lands.
    pub pending: Option<PendingAction>,
    /// Result of the last local action, success or failure.
    pub last_action: Option<ActionOutcome>,
}

/// Which local lifecycle action is currently in flight.
///
/// Kept as the identity of the action rather than a bare `busy` flag so the
/// panel can spin the one button that was pressed. With a flag, every button
/// spins, and "I pressed Stop and all three started working" is a worse
/// report of what is happening than no spinner at all.
///
/// Reload is absent on purpose: it goes over the session and its outcome
/// arrives as a gateway event, so there is no moment at which this side knows
/// it finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingAction {
    Start,
    Stop,
    Restart,
}

/// The outcome of the last start/stop/restart, shown inline in the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionOutcome {
    /// Whether the action did what it said.
    pub ok: bool,
    /// One line, already phrased for a human.
    pub message: String,
}

impl ActionOutcome {
    /// A successful outcome.
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
        }
    }

    /// A failed outcome.
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
        }
    }
}

impl Default for GatewayControlData {
    fn default() -> Self {
        Self {
            url: String::new(),
            connected: false,
            local: LocalDaemonState::Stopped,
            ssh_listen: String::new(),
            log_path: String::new(),
            vault_password_protected: false,
            pending: None,
            last_action: None,
        }
    }
}

impl GatewayControlData {
    /// Whether any local action is in flight.
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether this button should show a spinner: only the one that was
    /// pressed, not every disabled button on the panel.
    pub fn is_pending(&self, action: PendingAction) -> bool {
        self.pending == Some(action)
    }

    /// Whether the connected gateway is the daemon these controls manage.
    ///
    /// Loopback alone is not enough: the client could be pointed at a
    /// loopback address served by something other than the daemon named in
    /// the PID file, and the daemon could be down while the URL still says
    /// `127.0.0.1`. Requiring the daemon to be running keeps the claim to
    /// what the two facts together actually support.
    pub fn controls_connected_gateway(&self) -> bool {
        url_is_loopback(&self.url) && self.local.is_running()
    }

    /// Start is offered when nothing is running — including over a stale PID
    /// file, which `daemon::start` clears on its way through.
    pub fn can_start(&self) -> bool {
        !self.busy() && !self.local.is_running()
    }

    /// Stop is offered whenever there is something to stop, which includes a
    /// stale PID file: stopping it is how the file gets cleaned up.
    pub fn can_stop(&self) -> bool {
        !self.busy() && !matches!(self.local, LocalDaemonState::Stopped)
    }

    /// Restart always applies — it is stop-then-start, and the stop half
    /// tolerates a gateway that was not running.
    pub fn can_restart(&self) -> bool {
        !self.busy()
    }

    /// Reload runs over the wire, so it needs a live session and nothing else.
    /// It is the one control here that reaches a remote gateway.
    pub fn can_reload(&self) -> bool {
        !self.busy() && self.connected
    }

    /// The note explaining what the local controls do and do not reach.
    ///
    /// Returned as data rather than built in the renderer so the wording is
    /// pinned by a test — this is the sentence that stops someone from
    /// pressing "Stop" expecting it to act on a remote host.
    pub fn scope_note(&self) -> String {
        if url_is_loopback(&self.url) {
            "Start, stop and restart act on the gateway daemon on this machine \
             — the same one this client connects to over loopback."
                .to_string()
        } else {
            format!(
                "Start, stop and restart act on the gateway daemon on this machine, \
                 not on {}. Reload is the only control here that reaches a remote gateway.",
                if self.url.is_empty() {
                    "the remote host"
                } else {
                    &self.url
                }
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_urls_are_recognised_in_every_spelling_the_client_accepts() {
        for url in [
            "ssh://127.0.0.1:2222",
            "ssh://127.0.0.1",
            "ssh://127.9.9.9:2222",
            "ssh://user@127.0.0.1:2222",
            "ssh://localhost:2222",
            "ssh://LocalHost:2222",
            "ssh://[::1]:2222",
            "ws://127.0.0.1:9001",
        ] {
            assert!(url_is_loopback(url), "{url} should read as loopback");
        }
    }

    /// Pins the parser behaviour the implementation works around, so a
    /// future simplification back to matching only on `Host::Ipv4` fails here
    /// with an explanation rather than in the panel.
    #[test]
    fn ssh_urls_hand_back_a_dotted_quad_as_a_domain_not_an_ip() {
        let ssh = url::Url::parse("ssh://127.0.0.1:2222").expect("ssh url parses");
        assert!(
            matches!(ssh.host(), Some(url::Host::Domain("127.0.0.1"))),
            "ssh is a non-special scheme, so its host stays opaque: {:?}",
            ssh.host()
        );
        // A special scheme does resolve the address, which is why both arms
        // of the match need to be right.
        let ws = url::Url::parse("ws://127.0.0.1:9001").expect("ws url parses");
        assert!(
            matches!(ws.host(), Some(url::Host::Ipv4(_))),
            "{:?}",
            ws.host()
        );
    }

    #[test]
    fn everything_else_reads_as_remote() {
        for url in [
            "ssh://192.168.1.10:2222",
            "ssh://gateway.example.com:2222",
            // Resolves to loopback in practice, but only through a hosts
            // file we do not read. Under-claiming is the safe direction.
            "ssh://localhost.localdomain:2222",
            // A domain that merely starts with the magic name.
            "ssh://localhost.example.com:2222",
            "ssh://[2001:db8::1]:2222",
            "not a url at all",
            "",
        ] {
            assert!(!url_is_loopback(url), "{url} should read as remote");
        }
    }

    fn data(url: &str, local: LocalDaemonState, connected: bool) -> GatewayControlData {
        GatewayControlData {
            url: url.to_string(),
            connected,
            local,
            ..Default::default()
        }
    }

    #[test]
    fn start_is_offered_when_stopped_or_stale_and_withheld_while_running() {
        assert!(data("", LocalDaemonState::Stopped, false).can_start());
        assert!(data("", LocalDaemonState::Stale { pid: 7 }, false).can_start());
        assert!(!data("", LocalDaemonState::Running { pid: 7 }, false).can_start());
    }

    #[test]
    fn stop_is_offered_for_a_stale_pid_file_so_it_can_be_cleaned_up() {
        assert!(data("", LocalDaemonState::Stale { pid: 7 }, false).can_stop());
        assert!(data("", LocalDaemonState::Running { pid: 7 }, false).can_stop());
        assert!(!data("", LocalDaemonState::Stopped, false).can_stop());
    }

    #[test]
    fn reload_needs_a_connection_and_nothing_else() {
        assert!(!data("ssh://host:2222", LocalDaemonState::Stopped, false).can_reload());
        assert!(data("ssh://host:2222", LocalDaemonState::Stopped, true).can_reload());
        // Remote and disconnected locally is still reloadable: the command
        // travels over the session, not the PID file.
        assert!(data("ssh://remote:2222", LocalDaemonState::Stopped, true).can_reload());
    }

    #[test]
    fn a_busy_panel_offers_nothing() {
        let mut d = data(
            "ssh://127.0.0.1:2222",
            LocalDaemonState::Running { pid: 7 },
            true,
        );
        d.pending = Some(PendingAction::Stop);
        assert!(d.busy());
        assert!(!d.can_start());
        assert!(!d.can_stop());
        assert!(!d.can_restart());
        assert!(!d.can_reload());
    }

    #[test]
    fn only_the_pressed_button_reports_as_pending() {
        let mut d = data("", LocalDaemonState::Running { pid: 7 }, true);
        d.pending = Some(PendingAction::Stop);
        assert!(d.is_pending(PendingAction::Stop));
        assert!(!d.is_pending(PendingAction::Start));
        assert!(!d.is_pending(PendingAction::Restart));

        d.pending = None;
        assert!(!d.busy());
        assert!(!d.is_pending(PendingAction::Stop));
    }

    #[test]
    fn the_controls_only_claim_the_connected_gateway_when_both_facts_hold() {
        assert!(
            data(
                "ssh://127.0.0.1:2222",
                LocalDaemonState::Running { pid: 7 },
                true
            )
            .controls_connected_gateway()
        );
        // Loopback URL, but the daemon we manage is not the thing answering.
        assert!(
            !data("ssh://127.0.0.1:2222", LocalDaemonState::Stopped, true)
                .controls_connected_gateway()
        );
        // Our daemon is up, but the client is talking to someone else.
        assert!(
            !data(
                "ssh://gateway.example.com:2222",
                LocalDaemonState::Running { pid: 7 },
                true
            )
            .controls_connected_gateway()
        );
    }

    #[test]
    fn the_scope_note_names_the_remote_host_it_will_not_touch() {
        let remote = data(
            "ssh://gateway.example.com:2222",
            LocalDaemonState::Stopped,
            true,
        );
        let note = remote.scope_note();
        assert!(note.contains("ssh://gateway.example.com:2222"), "{note}");
        assert!(note.contains("not on"), "{note}");

        let local = data("ssh://127.0.0.1:2222", LocalDaemonState::Stopped, true);
        assert!(
            !local.scope_note().contains("not on"),
            "{}",
            local.scope_note()
        );
    }

    #[test]
    fn an_empty_url_still_produces_a_readable_note() {
        let note = data("", LocalDaemonState::Stopped, false).scope_note();
        assert!(note.contains("the remote host"), "{note}");
        assert!(!note.contains("not on ."), "{note}");
    }

    #[test]
    fn daemon_states_carry_their_pid_through_the_conversion() {
        use rustyclaw_core::daemon::DaemonStatus;
        assert_eq!(
            LocalDaemonState::from_status(&DaemonStatus::Running { pid: 42 }),
            LocalDaemonState::Running { pid: 42 }
        );
        assert_eq!(
            LocalDaemonState::from_status(&DaemonStatus::Stale { pid: 42 }),
            LocalDaemonState::Stale { pid: 42 }
        );
        assert_eq!(
            LocalDaemonState::from_status(&DaemonStatus::Stopped),
            LocalDaemonState::Stopped
        );
        assert_eq!(LocalDaemonState::Stale { pid: 42 }.pid(), Some(42));
        assert!(!LocalDaemonState::Stale { pid: 42 }.is_running());
    }
}
