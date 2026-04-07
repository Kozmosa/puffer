use crate::AppState;
use anyhow::Result;
use serde_json::json;
use serde_json::Value;
use std::path::Path;

/// Executes the Claude-compatible `StructuredOutput` workflow tool.
pub fn execute_structured_output(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    let _ = state;
    let _ = cwd;
    Ok(serde_json::to_string_pretty(&json!({
        "data": "Structured output provided successfully",
        "structured_output": input
    }))?)
}
