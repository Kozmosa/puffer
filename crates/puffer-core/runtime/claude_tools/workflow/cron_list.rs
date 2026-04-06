use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `CronList` tool scaffold.
pub fn execute_cron_list(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_cron_list(state, cwd, input)
}
