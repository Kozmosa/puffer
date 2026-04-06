use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `TodoWrite` tool scaffold.
pub fn execute_todo_write(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_todo_write(state, cwd, input)
}
