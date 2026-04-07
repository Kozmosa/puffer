use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const EDIT_DESCRIPTION: &str = "A tool for editing files";

#[derive(Debug, Deserialize)]
struct ClaudeEditInput {
    file_path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

/// Returns the Claude Code model-facing description for the `Edit` tool.
pub fn claude_edit_tool_description() -> &'static str {
    EDIT_DESCRIPTION
}

/// Returns the Claude Code model-facing JSON schema for the `Edit` tool input.
pub fn claude_edit_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "file_path": {
                "type": "string",
                "description": "The absolute path to the file to modify"
            },
            "old_string": {
                "type": "string",
                "description": "The text to replace"
            },
            "new_string": {
                "type": "string",
                "description": "The text to replace it with (must be different from old_string)"
            },
            "replace_all": {
                "type": "boolean",
                "description": "Replace all occurrences of old_string (default false)"
            }
        },
        "required": ["file_path", "old_string", "new_string"],
        "additionalProperties": false
    })
}

/// Executes a Claude-style `Edit` operation and returns a JSON tool result payload.
pub fn execute_claude_edit(_cwd: &Path, input: Value) -> Result<String> {
    let input: ClaudeEditInput = serde_json::from_value(input).context("invalid Edit input")?;

    let path = Path::new(&input.file_path);
    if !path.is_absolute() {
        bail!("Edit expects `file_path` to be an absolute path");
    }
    if input.old_string == input.new_string {
        bail!("No changes to make: old_string and new_string are exactly the same.");
    }

    let original = if path.exists() {
        fs::read_to_string(path)
            .with_context(|| format!("failed to read file {}", path.display()))?
    } else {
        String::new()
    };
    let (updated, original_file) = if !path.exists() && input.old_string.is_empty() {
        (input.new_string.clone(), String::new())
    } else if input.old_string.is_empty() {
        if !original.is_empty() {
            bail!("Edit with empty old_string requires the target file to be empty or missing.");
        }
        (input.new_string.clone(), original.clone())
    } else {
        let occurrences = occurrence_count(&original, &input.old_string);
        if occurrences == 0 {
            bail!(
                "Edit failed: old_string was not found in {}",
                path.display()
            );
        }
        if occurrences > 1 && !input.replace_all {
            bail!(
                "Edit failed: old_string is not unique in {}. Use replace_all or provide more context.",
                path.display()
            );
        }
        let updated = if input.replace_all {
            original.replace(&input.old_string, &input.new_string)
        } else {
            original.replacen(&input.old_string, &input.new_string, 1)
        };
        (updated, original.clone())
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory {}", parent.display()))?;
    }
    fs::write(path, &updated)
        .with_context(|| format!("failed to write file {}", path.display()))?;

    let output = json!({
        "filePath": input.file_path,
        "oldString": input.old_string,
        "newString": input.new_string,
        "originalFile": original_file,
        "structuredPatch": build_structured_patch(&original, &updated),
        "userModified": false,
        "replaceAll": input.replace_all
    });
    Ok(serde_json::to_string_pretty(&output)?)
}

/// Returns true when an Edit request targets an existing file mutation that
/// should require a prior full-file Read in the runtime dispatcher.
pub(crate) fn requires_prior_read(input: &Value) -> bool {
    let Some(file_path) = input.get("file_path").and_then(Value::as_str) else {
        return true;
    };
    let old_string = input
        .get("old_string")
        .and_then(Value::as_str)
        .unwrap_or_default();
    !(old_string.is_empty() && !Path::new(file_path).exists())
}

fn occurrence_count(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

fn build_structured_patch(original: &str, updated: &str) -> Vec<Value> {
    let old_lines = split_lines(original);
    let new_lines = split_lines(updated);
    vec![json!({
        "oldStart": if old_lines.is_empty() { 0 } else { 1 },
        "oldLines": old_lines.len(),
        "newStart": if new_lines.is_empty() { 0 } else { 1 },
        "newLines": new_lines.len(),
        "lines": old_lines
            .iter()
            .map(|line| format!("-{line}"))
            .chain(new_lines.iter().map(|line| format!("+{line}")))
            .collect::<Vec<_>>(),
    })]
}

fn split_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        Vec::new()
    } else {
        content.lines().map(str::to_string).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_uses_claude_field_names() {
        let schema = claude_edit_input_schema();
        let properties = schema["properties"].as_object().unwrap();
        assert!(properties.contains_key("file_path"));
        assert!(properties.contains_key("old_string"));
        assert!(properties.contains_key("new_string"));
        assert!(properties.contains_key("replace_all"));
        assert_eq!(
            schema["required"].as_array().unwrap().len(),
            3,
            "expected file_path, old_string, new_string to be required"
        );
    }

    #[test]
    fn edit_replaces_unique_occurrence() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("sample.txt");
        fs::write(&file, "alpha\nbeta\n").unwrap();
        let input = json!({
            "file_path": file.display().to_string(),
            "old_string": "beta",
            "new_string": "gamma"
        });

        let output = execute_claude_edit(temp.path(), input).unwrap();
        assert!(output.contains("\"replaceAll\": false"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha\ngamma\n");
    }

    #[test]
    fn edit_replace_all_updates_every_occurrence() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("sample.txt");
        fs::write(&file, "a\nx\nx\n").unwrap();
        let input = json!({
            "file_path": file.display().to_string(),
            "old_string": "x",
            "new_string": "y",
            "replace_all": true
        });

        let _ = execute_claude_edit(temp.path(), input).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "a\ny\ny\n");
    }

    #[test]
    fn edit_rejects_non_unique_without_replace_all() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("sample.txt");
        fs::write(&file, "x\nx\n").unwrap();
        let input = json!({
            "file_path": file.display().to_string(),
            "old_string": "x",
            "new_string": "y"
        });

        let error = execute_claude_edit(temp.path(), input).unwrap_err();
        assert!(error.to_string().contains("not unique"));
    }

    #[test]
    fn edit_rejects_relative_paths() {
        let temp = tempfile::tempdir().unwrap();
        let input = json!({
            "file_path": "relative.txt",
            "old_string": "x",
            "new_string": "y"
        });

        let error = execute_claude_edit(temp.path(), input).unwrap_err();
        assert!(error.to_string().contains("absolute path"));
    }

    #[test]
    fn edit_can_create_missing_file_with_empty_old_string() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("new.txt");
        let input = json!({
            "file_path": file.display().to_string(),
            "old_string": "",
            "new_string": "hello"
        });

        let output = execute_claude_edit(temp.path(), input).unwrap();
        assert!(output.contains("\"originalFile\": \"\""));
        assert_eq!(fs::read_to_string(&file).unwrap(), "hello");
    }

    #[test]
    fn missing_file_creation_edit_does_not_require_prior_read() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("new.txt");
        assert!(!requires_prior_read(&json!({
            "file_path": file.display().to_string(),
            "old_string": "",
            "new_string": "hello"
        })));
    }
}
