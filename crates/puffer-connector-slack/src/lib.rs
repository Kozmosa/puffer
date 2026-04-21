//! Slack connector for Puffer. Bridges inbound Slack messages into a
//! running Puffer process and sends the assistant's reply back to the
//! channel (or thread).
//!
//! The handler logic (slash commands, allow-list filtering, conversation
//! keying by `channel_id`/`thread_ts`) is a pure function and fully
//! covered by unit tests in [`handler`].
//!
//! The live Socket Mode driver is currently a compile-time stub — see
//! [`SlackConnector`] for the TODO describing the slack-morphism wiring
//! that will replace it.

mod config;
mod connector;
mod handler;

pub use config::SlackConfig;
pub use connector::SlackConnector;
pub use handler::{conversation_key, handle_command, handle_command_threaded, CommandOutcome};
