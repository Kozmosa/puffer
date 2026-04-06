use crate::AppState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `LSP` tool scaffold.
pub fn execute_lsp(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    super::support::execute_lsp(state, cwd, input)
}
