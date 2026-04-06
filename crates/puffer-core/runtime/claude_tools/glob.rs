use anyhow::{anyhow, bail, Context, Result};
use glob::Pattern;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_GLOB_LIMIT: usize = 100;

#[derive(Debug, Deserialize)]
struct ClaudeGlobInput {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct ClaudeGlobOutput {
    #[serde(rename = "durationMs")]
    duration_ms: u128,
    #[serde(rename = "numFiles")]
    num_files: usize,
    filenames: Vec<String>,
    truncated: bool,
}

/// Executes the Claude-compatible `Glob` tool over the current workspace.
///
/// The input shape matches Claude Code:
/// - `pattern` (required): glob pattern to match
/// - `path` (optional): directory to scope the search
///
/// Output matches Claude Code's shape:
/// - `durationMs`, `numFiles`, `filenames`, `truncated`
pub fn execute_claude_glob(cwd: &Path, input: Value) -> Result<String> {
    let started = Instant::now();
    let input: ClaudeGlobInput = serde_json::from_value(input).context("invalid Glob input")?;
    if input.pattern.trim().is_empty() {
        bail!("Glob pattern cannot be empty");
    }

    let pattern = Pattern::new(&input.pattern)
        .map_err(|error| anyhow!("invalid glob pattern `{}`: {error}", input.pattern))?;
    let root = input
        .path
        .as_deref()
        .map(|path| resolve_workspace_path(cwd, Path::new(path)))
        .transpose()?
        .unwrap_or_else(|| cwd.to_path_buf());
    if !root.exists() {
        bail!("Directory does not exist: {}", root.display());
    }
    if !root.is_dir() {
        bail!("Path is not a directory: {}", root.display());
    }

    let mut matches = Vec::new();
    collect_glob_matches(cwd, &root, &pattern, &mut matches)?;
    matches.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let truncated = matches.len() > DEFAULT_GLOB_LIMIT;
    let filenames = matches
        .into_iter()
        .take(DEFAULT_GLOB_LIMIT)
        .map(|(path, _)| path)
        .collect::<Vec<_>>();
    let output = ClaudeGlobOutput {
        duration_ms: started.elapsed().as_millis(),
        num_files: filenames.len(),
        filenames,
        truncated,
    };
    Ok(serde_json::to_string_pretty(&output)?)
}

fn collect_glob_matches(
    workspace_root: &Path,
    current: &Path,
    pattern: &Pattern,
    matches: &mut Vec<(String, u128)>,
) -> Result<()> {
    for entry in fs::read_dir(current)
        .with_context(|| format!("failed to list directory {}", current.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_glob_matches(workspace_root, &path, pattern, matches)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let relative = path.strip_prefix(workspace_root).unwrap_or(&path);
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        if pattern.matches(&relative_text) {
            matches.push((relative_text, file_mtime_ms(&path)));
        }
    }
    Ok(())
}

fn file_mtime_ms(path: &Path) -> u128 {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(system_time_to_epoch_ms)
        .unwrap_or(0)
}

fn system_time_to_epoch_ms(time: SystemTime) -> Option<u128> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|value| value.as_millis())
}

fn resolve_workspace_path(cwd: &Path, path: &Path) -> Result<PathBuf> {
    let workspace_root = fs::canonicalize(cwd)
        .with_context(|| format!("failed to resolve workspace root {}", cwd.display()))?;
    let workspace_path = normalize_path(cwd);
    let candidate = if path.is_absolute() {
        normalize_path(path)
    } else {
        normalize_path(&cwd.join(path))
    };
    if !candidate.starts_with(&workspace_path) {
        bail!(
            "path {} escapes workspace {}",
            path.display(),
            cwd.display()
        );
    }

    let ancestor = nearest_existing_ancestor(&candidate).ok_or_else(|| {
        anyhow!(
            "failed to resolve path {} inside workspace {}",
            path.display(),
            cwd.display()
        )
    })?;
    let canonical_ancestor = fs::canonicalize(&ancestor)
        .with_context(|| format!("failed to canonicalize {}", ancestor.display()))?;
    if !canonical_ancestor.starts_with(&workspace_root) {
        bail!(
            "path {} resolves through symlink outside workspace {}",
            path.display(),
            cwd.display()
        );
    }

    if candidate.exists() {
        let canonical_candidate = fs::canonicalize(&candidate)
            .with_context(|| format!("failed to canonicalize {}", candidate.display()))?;
        if !canonical_candidate.starts_with(&workspace_root) {
            bail!(
                "path {} resolves outside workspace {}",
                path.display(),
                cwd.display()
            );
        }
    }

    Ok(candidate)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn glob_returns_expected_shape() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(temp.path().join("src/lib.rs"), "pub fn x() {}\n").unwrap();

        let output = execute_claude_glob(
            temp.path(),
            json!({
                "pattern": "src/*.rs"
            }),
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["truncated"], false);
        assert_eq!(parsed["numFiles"], 2);
        assert_eq!(parsed["filenames"][0], "src/lib.rs");
        assert_eq!(parsed["filenames"][1], "src/main.rs");
    }

    #[test]
    fn glob_sorts_by_mtime_descending() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("older.txt"), "1").unwrap();
        std::thread::sleep(Duration::from_millis(5));
        fs::write(temp.path().join("newer.txt"), "2").unwrap();

        let output = execute_claude_glob(
            temp.path(),
            json!({
                "pattern": "*.txt"
            }),
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["filenames"][0], "newer.txt");
        assert_eq!(parsed["filenames"][1], "older.txt");
    }

    #[test]
    fn glob_rejects_paths_outside_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let error = execute_claude_glob(
            temp.path(),
            json!({
                "pattern": "*.rs",
                "path": "../"
            }),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("escapes workspace"));
    }
}
