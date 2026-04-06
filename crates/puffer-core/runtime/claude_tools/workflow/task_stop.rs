use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `TaskStop` tool scaffold.
pub fn execute_task_stop(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_task_stop(state, cwd, input)
}
