use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `ExitPlanMode` tool scaffold.
pub fn execute_exit_plan_mode(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_exit_plan_mode(state, cwd, input)
}
