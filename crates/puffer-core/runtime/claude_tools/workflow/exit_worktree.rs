use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `ExitWorktree` tool scaffold.
pub fn execute_exit_worktree(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    super::support::execute_exit_worktree(state, cwd, input)
}
