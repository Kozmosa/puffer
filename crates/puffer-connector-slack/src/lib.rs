//! Slack connector for Puffer. Bridges inbound Slack messages into a
//! running Puffer process and sends the assistant's reply back to the
//! channel (or thread).
//!
//! See [`puffer_connector_core`] for the shared conversation→session
//! bridge and the built-in `/help`/`/new`/`/status`/`/usage` commands;
//! this crate layers on Slack-specific concerns:
//! * bot-self filtering so we never loop on our own outgoing messages
//! * `<@BOT>` mention detection for channels (mention-gating in groups)
//! * thread-scoped sessions: inbound messages with a `thread_ts` key by
//!   `{channel_id}:{thread_ts}` so a single channel can host multiple
//!   parallel Puffer conversations
//! * [`MessageSplitter::SLACK`](puffer_connector_core::MessageSplitter::SLACK)
//!   chunking for long replies
//!
//! The live Socket Mode driver is a compile-time stub — see
//! [`SlackConnector`] for the v2 plan describing the slack-morphism
//! wiring that will replace it.

mod config;
mod connector;
mod handler;

pub use config::SlackConfig;
pub use connector::SlackConnector;
pub use handler::handle_command;
pub use puffer_connector_core::{CommandOutcome, InboundMessage};
