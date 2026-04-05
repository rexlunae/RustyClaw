//! Concrete [`ChatListener`](crate::server::ChatListener) implementations.

mod irc;

#[cfg(feature = "tls")]
mod tls_irc;

pub use irc::IrcListener;

#[cfg(feature = "tls")]
pub use tls_irc::TlsIrcListener;
