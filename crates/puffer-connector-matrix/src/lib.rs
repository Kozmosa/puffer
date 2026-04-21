//! Matrix connector for Puffer. Bridges inbound Matrix room messages into
//! a running Puffer process and sends the assistant's reply back to the
//! room.
//!
//! Backed by [`matrix-sdk`](https://docs.rs/matrix-sdk). The connector:
//! * logs in to a homeserver with username/password
//! * syncs for new room events on a background thread
//! * maps each Matrix `room_id` to a Puffer session (created on first
//!   message)
//! * supports `/new` to reset the session, `/help` for usage, any other
//!   text is dispatched to the agent
//! * restricts access to `allowed_users` (Matrix user ids such as
//!   `@alice:example.org`) when configured
//!
//! E2EE rooms are intentionally out of scope for v1: the `matrix-sdk`
//! default features (sqlite + olm) are disabled in `Cargo.toml` to keep
//! the build lean. Supporting encrypted rooms would re-enable the
//! `e2e-encryption` / `sqlite` features and layer a crypto store on top.

mod config;
mod connector;
mod handler;

pub use config::MatrixConfig;
pub use connector::MatrixConnector;
pub use handler::{handle_command, CommandOutcome};
