//! `Remember` workflow tool: lets the agent persist durable facts/preferences
//! about the user into the global keyed-block store (`~/.puffer/user.md`, see
//! [`crate::user_memory`]). Silent write (auto-approved); the call is recorded
//! in the session transcript, so every change is auditable + reversible.

use crate::user_memory::{normalize_key, UserMemory};
use crate::AppState;
use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Deserialize)]
struct RememberInput {
    action: String,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    value: Option<String>,
}

/// Dispatched from `execute_workflow_tool` for tool id `Remember`.
pub fn execute_remember(_state: &mut AppState, _cwd: &Path, input: Value) -> Result<String> {
    let input: RememberInput = serde_json::from_value(input)?;
    let memory = UserMemory::global()?;
    let key = input.key.unwrap_or_default();
    let value = input.value.unwrap_or_default();

    let payload = match input.action.as_str() {
        "set" | "remember" => match memory.set(&key, &value) {
            Ok(block) => json!({
                "success": true, "action": "set", "key": block.key,
                "message": format!("Remembered `{}`.", block.key),
            }),
            Err(error) => json!({ "success": false, "action": "set", "error": error.to_string() }),
        },
        "append" => match memory.append(&key, &value) {
            Ok(block) => json!({
                "success": true, "action": "append", "key": block.key,
                "message": format!("Updated `{}`.", block.key),
            }),
            Err(error) => json!({ "success": false, "action": "append", "error": error.to_string() }),
        },
        "forget" | "delete" => match memory.delete(&key) {
            Ok(removed) => json!({
                "success": removed, "action": "forget", "key": normalize_key(&key),
                "message": if removed { "Forgotten." } else { "No matching fact." },
            }),
            Err(error) => json!({ "success": false, "action": "forget", "error": error.to_string() }),
        },
        "list" => match memory.list() {
            Ok(facts) => json!({ "success": true, "action": "list", "facts": facts }),
            Err(error) => json!({ "success": false, "action": "list", "error": error.to_string() }),
        },
        other => json!({
            "success": false,
            "error": format!("unknown action `{other}`; use set, append, forget, or list"),
        }),
    };
    Ok(payload.to_string())
}
