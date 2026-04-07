use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `AskUserQuestion` tool scaffold.
pub fn execute_ask_user_question(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_ask_user_question(state, cwd, input)
}
