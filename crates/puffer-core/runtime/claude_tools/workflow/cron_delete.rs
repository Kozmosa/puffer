use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `CronDelete` tool scaffold.
pub fn execute_cron_delete(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_cron_delete(state, cwd, input)
}
