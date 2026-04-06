use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `TaskOutput` tool scaffold.
pub fn execute_task_output(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    super::support::execute_task_output(state, cwd, input)
}
