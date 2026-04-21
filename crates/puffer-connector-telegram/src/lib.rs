//! Telegram connector for Puffer. Bridges inbound chat messages into a
//! running Puffer process and sends the assistant's reply back to the
//! user.
//!
//! Backed by [`teloxide`](https://docs.rs/teloxide) for the bot SDK. The
//! connector:
//! * polls Telegram for new updates on a background thread
//! * maps each `chat_id` to a Puffer session (created on first message)
//! * supports `/new` to reset the session, `/help` for usage, any other
//!   text is dispatched to the agent
//! * restricts access to `allowed_users` if configured

mod config;
mod connector;
mod handler;

pub use config::TelegramConfig;
pub use connector::TelegramConnector;
pub use handler::{handle_command, CommandOutcome};
