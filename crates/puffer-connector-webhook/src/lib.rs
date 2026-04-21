//! HTTP webhook connector for Puffer. Exposes a small axum-backed HTTP
//! surface so any upstream system that can `POST` JSON (n8n, Zapier,
//! cURL, a frontend, another service, …) can talk to a running Puffer
//! process.
//!
//! The connector:
//! * binds to a caller-configured `bind_address`
//! * accepts `POST {path}` (default `/puffer`) with a JSON body of
//!   `{"conversation_id": "...", "message": "..."}`
//! * optionally validates an `Authorization: Bearer <token>` header
//!   against a configured `auth_token`
//! * maps each `conversation_id` to a Puffer session (created on first
//!   message) — the mapping is platform-scoped to `"webhook"`
//! * supports `/new` (reset), `/start`, `/help` as local slash
//!   commands, mirroring the Telegram connector
//! * exposes `GET /health` returning `200 OK "puffer"`
//!
//! The axum layer is deliberately thin — all decision logic lives in
//! [`handle_command`] so it is exercised by unit tests without needing
//! to stand up a TCP listener.

pub mod config;
pub mod connector;
pub mod handler;
pub mod router;

pub use config::WebhookConfig;
pub use connector::WebhookConnector;
pub use handler::{handle_command, CommandOutcome, PLATFORM_ID};
pub use router::{build_router, AppState};
