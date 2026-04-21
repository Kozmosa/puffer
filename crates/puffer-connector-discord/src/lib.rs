//! Discord connector for Puffer. Bridges inbound chat messages into a
//! running Puffer process and sends the assistant's reply back to the
//! user.
//!
//! Backed by [`serenity`](https://docs.rs/serenity) for the bot SDK. The
//! connector:
//! * connects to Discord's gateway on a background thread
//! * maps each `channel_id` to a Puffer session (created on first message)
//! * supports `/new` to reset the session, `/help` for usage, any other
//!   text is dispatched to the agent
//! * restricts access to `allowed_users` if configured

mod config;
mod connector;
mod handler;

pub use config::DiscordConfig;
pub use connector::DiscordConnector;
pub use handler::{handle_command, CommandOutcome};
