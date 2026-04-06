use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `EnterWorktree` tool scaffold.
pub fn execute_enter_worktree(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    super::support::execute_enter_worktree(state, cwd, input)
}
