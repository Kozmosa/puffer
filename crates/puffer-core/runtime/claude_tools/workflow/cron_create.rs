use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `CronCreate` tool scaffold.
pub fn execute_cron_create(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_cron_create(state, cwd, input)
}
