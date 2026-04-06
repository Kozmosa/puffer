use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;

/// Claude-compatible input payload for the `Bash` tool.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ClaudeBashInput {
    pub command: String,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub run_in_background: bool,
    #[serde(default, rename = "dangerouslyDisableSandbox")]
    pub dangerously_disable_sandbox: bool,
}

/// Claude-compatible output payload for the `Bash` tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeBashOutput {
    pub stdout: String,
    pub stderr: String,
    pub interrupted: bool,
    #[serde(skip_serializing_if = "Option::is_none", rename = "backgroundTaskId")]
    pub background_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "backgroundedByUser")]
    pub backgrounded_by_user: Option<bool>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "assistantAutoBackgrounded"
    )]
    pub assistant_auto_backgrounded: Option<bool>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "dangerouslyDisableSandbox"
    )]
    pub dangerously_disable_sandbox: Option<bool>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "returnCodeInterpretation"
    )]
    pub return_code_interpretation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "noOutputExpected")]
    pub no_output_expected: Option<bool>,
}

/// Normalized result envelope for one Claude-style Bash execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeBashExecution {
    pub success: bool,
    pub output: ClaudeBashOutput,
}

/// Returns the model-facing description text used for one `Bash` invocation.
pub fn tool_description(input: &ClaudeBashInput) -> String {
    input
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Run shell command".to_string())
}

/// Parses JSON input and executes a Claude-style `Bash` tool invocation.
pub fn execute_from_value(cwd: &Path, input: Value) -> Result<ClaudeBashExecution> {
    let typed: ClaudeBashInput =
        serde_json::from_value(input).context("invalid Bash tool input payload")?;
    execute(cwd, typed)
}

/// Executes a Claude-style `Bash` tool invocation in the provided working directory.
pub fn execute(cwd: &Path, input: ClaudeBashInput) -> Result<ClaudeBashExecution> {
    if input.run_in_background {
        return execute_background(cwd, input);
    }
    execute_foreground(cwd, input)
}

fn execute_background(cwd: &Path, input: ClaudeBashInput) -> Result<ClaudeBashExecution> {
    let child = Command::new("bash")
        .arg("-lc")
        .arg(&input.command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| {
            format!(
                "failed to start background bash command in {}",
                cwd.display()
            )
        })?;
    let task_id = format!("shell-{}", child.id());
    Ok(ClaudeBashExecution {
        success: true,
        output: ClaudeBashOutput {
            stdout: String::new(),
            stderr: String::new(),
            interrupted: false,
            background_task_id: Some(task_id),
            backgrounded_by_user: Some(false),
            assistant_auto_backgrounded: Some(false),
            dangerously_disable_sandbox: Some(input.dangerously_disable_sandbox),
            return_code_interpretation: None,
            no_output_expected: Some(true),
        },
    })
}

fn execute_foreground(cwd: &Path, input: ClaudeBashInput) -> Result<ClaudeBashExecution> {
    let timeout_ms = input
        .timeout
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(1, MAX_TIMEOUT_MS);
    let command = input.command.clone();
    let timed = run_bash_command(cwd, &command, timeout_ms)?;
    let mut stderr = String::from_utf8_lossy(&timed.output.stderr).to_string();
    if timed.timed_out {
        if !stderr.trim().is_empty() {
            stderr.push('\n');
        }
        stderr.push_str(&format!("command timed out after {timeout_ms}ms"));
    }
    let stdout = String::from_utf8_lossy(&timed.output.stdout).to_string();
    let success = timed.output.status.success() && !timed.timed_out;
    let no_output_expected = success && stdout.trim().is_empty() && stderr.trim().is_empty();
    Ok(ClaudeBashExecution {
        success,
        output: ClaudeBashOutput {
            stdout,
            stderr,
            interrupted: timed.timed_out,
            background_task_id: None,
            backgrounded_by_user: None,
            assistant_auto_backgrounded: None,
            dangerously_disable_sandbox: Some(input.dangerously_disable_sandbox),
            return_code_interpretation: classify_return_code(timed.output.status.code()),
            no_output_expected: Some(no_output_expected),
        },
    })
}

fn classify_return_code(code: Option<i32>) -> Option<String> {
    match code {
        Some(130) => Some("interrupted_by_signal".to_string()),
        Some(137) => Some("killed".to_string()),
        _ => None,
    }
}

struct TimedCommandOutput {
    output: Output,
    timed_out: bool,
}

fn run_bash_command(cwd: &Path, command: &str, timeout_ms: u64) -> Result<TimedCommandOutput> {
    let mut child = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to execute bash command in {}", cwd.display()))?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if child
            .try_wait()
            .with_context(|| format!("failed to poll bash command in {}", cwd.display()))?
            .is_some()
        {
            let output = child
                .wait_with_output()
                .with_context(|| format!("failed to collect bash output in {}", cwd.display()))?;
            return Ok(TimedCommandOutput {
                output,
                timed_out: false,
            });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().with_context(|| {
                format!(
                    "failed to collect timed-out bash output in {}",
                    cwd.display()
                )
            })?;
            return Ok(TimedCommandOutput {
                output,
                timed_out: true,
            });
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Builds a human-readable summary line for UI/status displays.
pub fn summary_line(input: &ClaudeBashInput) -> Result<String> {
    if input.command.trim().is_empty() {
        return Err(anyhow!("Bash command cannot be empty"));
    }
    Ok(tool_description(input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn description_uses_fallback_when_omitted() {
        let input = ClaudeBashInput {
            command: "echo hi".to_string(),
            timeout: None,
            description: None,
            run_in_background: false,
            dangerously_disable_sandbox: false,
        };
        assert_eq!(tool_description(&input), "Run shell command");
    }

    #[test]
    fn description_prefers_explicit_value() {
        let input = ClaudeBashInput {
            command: "echo hi".to_string(),
            timeout: None,
            description: Some("Show greeting".to_string()),
            run_in_background: false,
            dangerously_disable_sandbox: false,
        };
        assert_eq!(tool_description(&input), "Show greeting");
    }

    #[test]
    fn execute_foreground_returns_stdout() {
        let temp = tempfile::tempdir().unwrap();
        let result = execute(
            temp.path(),
            ClaudeBashInput {
                command: "printf 'hello'".to_string(),
                timeout: Some(1_000),
                description: None,
                run_in_background: false,
                dangerously_disable_sandbox: false,
            },
        )
        .unwrap();
        assert!(result.success);
        assert_eq!(result.output.stdout, "hello");
        assert!(!result.output.interrupted);
        assert_eq!(result.output.dangerously_disable_sandbox, Some(false));
    }

    #[test]
    fn execute_timeout_marks_interrupted() {
        let temp = tempfile::tempdir().unwrap();
        let result = execute(
            temp.path(),
            ClaudeBashInput {
                command: "sleep 0.2".to_string(),
                timeout: Some(20),
                description: None,
                run_in_background: false,
                dangerously_disable_sandbox: true,
            },
        )
        .unwrap();
        assert!(!result.success);
        assert!(result.output.interrupted);
        assert!(result.output.stderr.contains("timed out after"));
        assert_eq!(result.output.dangerously_disable_sandbox, Some(true));
    }

    #[test]
    fn execute_background_returns_task_id() {
        let temp = tempfile::tempdir().unwrap();
        let result = execute(
            temp.path(),
            ClaudeBashInput {
                command: "sleep 0.1".to_string(),
                timeout: Some(1_000),
                description: None,
                run_in_background: true,
                dangerously_disable_sandbox: false,
            },
        )
        .unwrap();
        assert!(result.success);
        assert!(result.output.background_task_id.is_some());
        assert_eq!(result.output.backgrounded_by_user, Some(false));
        assert_eq!(result.output.assistant_auto_backgrounded, Some(false));
    }

    #[test]
    fn execute_from_value_parses_claude_field_names() {
        let temp = tempfile::tempdir().unwrap();
        let input = json!({
            "command": "printf ok",
            "timeout": 1000,
            "description": "Print test token",
            "run_in_background": false,
            "dangerouslyDisableSandbox": true
        });
        let result = execute_from_value(temp.path(), input).unwrap();
        assert!(result.success);
        assert_eq!(result.output.stdout, "ok");
        assert_eq!(result.output.dangerously_disable_sandbox, Some(true));
    }

    #[test]
    fn summary_line_rejects_empty_commands() {
        let input = ClaudeBashInput {
            command: "   ".to_string(),
            timeout: None,
            description: None,
            run_in_background: false,
            dangerously_disable_sandbox: false,
        };
        let error = summary_line(&input).unwrap_err();
        assert!(error.to_string().contains("cannot be empty"));
    }
}
