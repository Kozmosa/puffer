use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `TeamCreate` tool scaffold.
pub fn execute_team_create(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_team_create(state, cwd, input)
}
