use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `SendUserMessage` tool scaffold.
pub fn execute_send_user_message(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    super::support::execute_send_user_message(state, cwd, input)
}
