use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `PowerShell` tool scaffold.
pub fn execute_powershell(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_powershell(state, cwd, input)
}
