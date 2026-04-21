//! Email connector for Puffer. Bridges inbound email messages into a
//! running Puffer process and sends the assistant's reply back to the
//! sender.
//!
//! Backed by [`async-imap`](https://docs.rs/async-imap) for inbound
//! polling and [`lettre`](https://docs.rs/lettre) for SMTP replies. The
//! connector:
//! * polls an IMAP mailbox every `poll_interval_secs` for UNSEEN mail on
//!   a background thread
//! * maps each email thread (by first `Message-ID`) to a Puffer session
//! * supports `/new` to reset the session, `/help` for usage, any other
//!   body text is dispatched to the agent
//! * restricts access to `allowed_senders` if configured (matched
//!   case-insensitively on the `From:` address)

mod config;
mod connector;
mod handler;

pub use config::EmailConfig;
pub use connector::EmailConnector;
pub use handler::{handle_command, CommandOutcome};
