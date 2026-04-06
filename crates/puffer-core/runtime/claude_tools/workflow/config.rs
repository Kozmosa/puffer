use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `Config` tool scaffold.
pub fn execute_config(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_config(state, cwd, input)
}
