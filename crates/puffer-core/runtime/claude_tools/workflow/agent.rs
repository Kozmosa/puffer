use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `Agent` tool scaffold.
pub fn execute_agent(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_agent(state, cwd, input)
}
