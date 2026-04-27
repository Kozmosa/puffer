use crate::AppState;
use anyhow::Result;
use puffer_resources::LoadedResources;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `LSP` tool using a real stdio LSP session.
pub fn execute_lsp(
    _state: &mut AppState,
    resources: &LoadedResources,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    super::lsp_live::execute_lsp(resources, cwd, input)
}

/// Executes an LSP query without requiring an application session state.
pub fn execute_lsp_query(resources: &LoadedResources, cwd: &Path, input: Value) -> Result<String> {
    super::lsp_live::execute_lsp(resources, cwd, input)
}

/// Shuts down cached LSP services for the current process.
pub fn shutdown_lsp_services() -> Result<()> {
    super::lsp_live::shutdown_lsp_services()
}
