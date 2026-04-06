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
    if input.old_string.is_empty() {
        bail!("Edit requires `old_string` to be non-empty");
    }
    if input.old_string == input.new_string {
        bail!("No changes to make: old_string and new_string are exactly the same.");
    }

    let original = fs::read_to_string(path)
        .with_context(|| format!("failed to read file {}", path.display()))?;
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
    fs::write(path, &updated)
        .with_context(|| format!("failed to write file {}", path.display()))?;

    let output = json!({
        "filePath": input.file_path,
        "oldString": input.old_string,
        "newString": input.new_string,
        "originalFile": original,
        "structuredPatch": [{
            "oldStart": 1,
            "oldLines": line_count(&original),
            "newStart": 1,
            "newLines": line_count(&updated),
            "lines": []
        }],
        "userModified": false,
        "replaceAll": input.replace_all
    });
    Ok(serde_json::to_string_pretty(&output)?)
}

fn occurrence_count(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

fn line_count(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count()
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
}
