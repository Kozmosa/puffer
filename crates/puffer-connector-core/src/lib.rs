//! Core runtime glue for external "connectors" (Telegram, Slack, …) that
//! bridge an outside platform into a running Puffer process.
//!
//! A connector is a background service that:
//! 1. Listens for inbound messages on its platform.
//! 2. Maps each external conversation (channel/DM/thread) to a Puffer session,
//!    creating one on first contact and resuming it on subsequent messages.
//! 3. Forwards the message text through [`ConnectorRuntime::dispatch`],
//!    which runs one Puffer turn against the stored session and returns
//!    the assistant's final reply.
//! 4. Sends that reply back on the platform.
//!
//! This crate is deliberately framework-agnostic: it exposes a plain
//! blocking [`ConnectorRuntime::dispatch`] function plus the persistent
//! conversation→session map. Platform-specific crates own their own
//! runtime (e.g. Tokio for HTTP-based bots) and call `dispatch` via
//! `spawn_blocking` when they need to keep the async event loop free.

mod config;
mod runtime;
mod session_map;
mod traits;

pub use config::{ConnectorConfig, ConnectorsConfig};
pub use runtime::{ConnectorRuntime, ConnectorRuntimeConfig, DispatchOutcome};
pub use session_map::{ConversationKey, ConversationSessionMap};
pub use traits::{Connector, ConnectorHandle, ConnectorStartError};
