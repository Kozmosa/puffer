use crate::state::ClaudeReadState;
use crate::AppState;
use anyhow::{bail, Context, Result};
use puffer_provider_openai::OpenAIRequestConfig;
use puffer_resources::LoadedResources;
use puffer_tools::{ToolDefinition, ToolExecutionResult, ToolOutput, ToolRegistry};
use puffer_transport_anthropic::AnthropicRequestConfig;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

mod bash;
mod edit;
mod glob;
mod grep;
mod mcp_resources;
mod notebook_edit;
mod read;
mod skill;
mod tool_search;
mod web_fetch;
mod web_search;
mod workflow;
mod write;

/// Carries provider-specific execution context for runtime-backed tools.
pub(crate) enum ProviderToolContext<'a> {
    None,
    OpenAI {
        request_config: &'a OpenAIRequestConfig,
        model_id: &'a str,
    },
    Anthropic {
        request_config: &'a AnthropicRequestConfig,
        model_id: &'a str,
    },
}

/// Returns true when the handler should be routed through the Claude tool dispatcher.
pub(crate) fn is_claude_runtime_handler(handler: &str) -> bool {
    handler.starts_with("runtime:claude_") || handler.starts_with("runtime:workflow:")
}

/// Executes one tool invocation using the Claude parity dispatcher when applicable.
pub(crate) fn execute_tool(
    state: &mut AppState,
    resources: &LoadedResources,
    registry: &ToolRegistry,
    definition: &ToolDefinition,
    cwd: &Path,
    input: Value,
    provider_context: ProviderToolContext<'_>,
) -> Result<ToolExecutionResult> {
    match definition.id.as_str() {
        "Bash" => {
            let execution = bash::execute_from_value(cwd, input)?;
            let output = serde_json::to_string_pretty(&execution.output)
                .context("failed to serialize Bash output")?;
            Ok(tool_result(definition, execution.success, output))
        }
        "Read" => {
            let output = read::execute_claude_read_tool(cwd, input.clone())?;
            record_read_from_input(state, &input)?;
            Ok(tool_result(definition, true, output))
        }
        "Write" => {
            let mut read_state = clone_read_state(state);
            let output = write::execute_claude_write_tool(input.clone(), &mut read_state)?;
            sync_read_state(state, read_state);
            if let Some(path) = input_file_path(&input, "file_path")? {
                mark_fully_read(state, &path)?;
            }
            Ok(tool_result(definition, true, output))
        }
        "Edit" => {
            if edit::requires_prior_read(&input) {
                enforce_read_precondition(state, input_file_path(&input, "file_path")?.as_deref())?;
            }
            let output = edit::execute_claude_edit(cwd, input.clone())?;
            if let Some(path) = input_file_path(&input, "file_path")? {
                mark_fully_read(state, &path)?;
            }
            Ok(tool_result(definition, true, output))
        }
        "Glob" => Ok(tool_result(
            definition,
            true,
            glob::execute_claude_glob(cwd, input)?,
        )),
        "Grep" => Ok(tool_result(
            definition,
            true,
            grep::execute_claude_grep(cwd, input)?,
        )),
        "NotebookEdit" => {
            enforce_read_precondition(state, input_file_path(&input, "notebook_path")?.as_deref())?;
            let output = notebook_edit::execute_notebook_edit_tool(cwd, input.clone())?;
            if let Some(path) = input_file_path(&input, "notebook_path")? {
                mark_fully_read(state, &path)?;
            }
            Ok(tool_result(definition, true, output))
        }
        "Skill" => Ok(tool_result(
            definition,
            true,
            skill::execute_claude_skill_tool(resources, input)?,
        )),
        "ToolSearch" => Ok(tool_result(
            definition,
            true,
            tool_search::execute_claude_tool_search_tool(registry, input)?,
        )),
        "ListMcpResourcesTool" | "ReadMcpResourceTool" => Ok(tool_result(
            definition,
            true,
            super::local_tools::execute_runtime_local_tool(
                resources, registry, definition, cwd, input,
            )?,
        )),
        "WebFetch" => {
            let output = serde_json::to_string_pretty(&web_fetch::execute_claude_web_fetch(input)?)
                .context("failed to serialize WebFetch output")?;
            Ok(tool_result(definition, true, output))
        }
        "WebSearch" => {
            let output = match provider_context {
                ProviderToolContext::OpenAI {
                    request_config,
                    model_id,
                } => web_search::execute_claude_openai_web_search(request_config, model_id, input)?,
                ProviderToolContext::Anthropic {
                    request_config,
                    model_id,
                } => web_search::execute_claude_anthropic_web_search(
                    request_config,
                    model_id,
                    input,
                )?,
                ProviderToolContext::None => {
                    bail!("WebSearch requires provider execution context")
                }
            };
            Ok(tool_result(definition, true, output))
        }
        _ if definition.handler.starts_with("runtime:workflow:") => Ok(tool_result(
            definition,
            true,
            execute_workflow_tool(state, cwd, definition.id.as_str(), input)?,
        )),
        _ if super::local_tools::is_runtime_local_tool(definition) => Ok(tool_result(
            definition,
            true,
            super::local_tools::execute_runtime_local_tool(
                resources, registry, definition, cwd, input,
            )?,
        )),
        _ => registry.execute_json(&definition.id, cwd, input),
    }
}

fn tool_result(definition: &ToolDefinition, success: bool, stdout: String) -> ToolExecutionResult {
    ToolExecutionResult {
        tool_id: definition.id.clone(),
        success,
        output: ToolOutput {
            stdout,
            stderr: String::new(),
            metadata: Value::Null,
        },
    }
}

fn input_file_path(input: &Value, field: &str) -> Result<Option<PathBuf>> {
    Ok(input.get(field).and_then(Value::as_str).map(PathBuf::from))
}

fn clone_read_state(state: &AppState) -> HashMap<PathBuf, write::ClaudeReadSnapshot> {
    state
        .claude_read_state
        .iter()
        .map(|(path, snapshot)| {
            (
                path.clone(),
                write::ClaudeReadSnapshot {
                    timestamp_ms: snapshot.timestamp_ms,
                    is_partial_view: snapshot.is_partial_view,
                },
            )
        })
        .collect()
}

fn sync_read_state(state: &mut AppState, read_state: HashMap<PathBuf, write::ClaudeReadSnapshot>) {
    state.claude_read_state = read_state
        .into_iter()
        .map(|(path, snapshot)| {
            (
                path,
                ClaudeReadState {
                    timestamp_ms: snapshot.timestamp_ms,
                    is_partial_view: snapshot.is_partial_view,
                },
            )
        })
        .collect();
}

fn record_read_from_input(state: &mut AppState, input: &Value) -> Result<()> {
    let Some(path) = input_file_path(input, "file_path")? else {
        return Ok(());
    };
    let timestamp_ms = file_timestamp_ms(&path)?;
    let is_partial_view = input.get("offset").is_some()
        || input.get("limit").is_some()
        || input.get("pages").is_some();
    state.claude_read_state.insert(
        path,
        ClaudeReadState {
            timestamp_ms,
            is_partial_view,
        },
    );
    Ok(())
}

fn mark_fully_read(state: &mut AppState, path: &Path) -> Result<()> {
    let timestamp_ms = file_timestamp_ms(path)?;
    state.claude_read_state.insert(
        path.to_path_buf(),
        ClaudeReadState {
            timestamp_ms,
            is_partial_view: false,
        },
    );
    Ok(())
}

fn enforce_read_precondition(state: &AppState, path: Option<&Path>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    let Some(snapshot) = state.claude_read_state.get(path) else {
        bail!("File has not been read yet. Read it first before writing to it.");
    };
    if snapshot.is_partial_view {
        bail!("File has not been read yet. Read it first before writing to it.");
    }
    let timestamp_ms = file_timestamp_ms(path)?;
    if timestamp_ms > snapshot.timestamp_ms {
        bail!(
            "File has been modified since read, either by the user or by a linter. Read it again before attempting to write it."
        );
    }
    Ok(())
}

fn file_timestamp_ms(path: &Path) -> Result<u128> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat file {}", path.display()))?;
    let modified = metadata
        .modified()
        .with_context(|| format!("failed to read mtime for {}", path.display()))?;
    let duration = modified
        .duration_since(UNIX_EPOCH)
        .with_context(|| format!("mtime for {} predates UNIX_EPOCH", path.display()))?;
    Ok(duration.as_millis())
}

fn execute_workflow_tool(
    state: &mut AppState,
    cwd: &Path,
    tool_id: &str,
    input: Value,
) -> Result<String> {
    match tool_id {
        "Agent" => workflow::agent::execute_agent(state, cwd, input),
        "AskUserQuestion" => {
            workflow::ask_user_question::execute_ask_user_question(state, cwd, input)
        }
        "Config" => workflow::config::execute_config(state, cwd, input),
        "CronCreate" => workflow::cron_create::execute_cron_create(state, cwd, input),
        "CronDelete" => workflow::cron_delete::execute_cron_delete(state, cwd, input),
        "CronList" => workflow::cron_list::execute_cron_list(state, cwd, input),
        "EnterPlanMode" => workflow::enter_plan_mode::execute_enter_plan_mode(state, cwd, input),
        "EnterWorktree" => workflow::enter_worktree::execute_enter_worktree(state, cwd, input),
        "ExitPlanMode" => workflow::exit_plan_mode::execute_exit_plan_mode(state, cwd, input),
        "ExitWorktree" => workflow::exit_worktree::execute_exit_worktree(state, cwd, input),
        "LSP" => workflow::lsp::execute_lsp(state, cwd, input),
        "PowerShell" => workflow::powershell::execute_powershell(state, cwd, input),
        "SendMessage" => workflow::send_message::execute_send_message(state, cwd, input),
        "SendUserMessage" | "Brief" => {
            workflow::send_user_message::execute_send_user_message(state, cwd, input)
        }
        "StructuredOutput" => {
            workflow::structured_output::execute_structured_output(state, cwd, input)
        }
        "TaskCreate" => workflow::task_create::execute_task_create(state, cwd, input),
        "TaskGet" => workflow::task_get::execute_task_get(state, cwd, input),
        "TaskList" => workflow::task_list::execute_task_list(state, cwd, input),
        "TaskOutput" => workflow::task_output::execute_task_output(state, cwd, input),
        "TaskStop" => workflow::task_stop::execute_task_stop(state, cwd, input),
        "TaskUpdate" => workflow::task_update::execute_task_update(state, cwd, input),
        "TeamCreate" => workflow::team_create::execute_team_create(state, cwd, input),
        "TeamDelete" => workflow::team_delete::execute_team_delete(state, cwd, input),
        "TodoWrite" => workflow::todo_write::execute_todo_write(state, cwd, input),
        other => bail!("workflow tool `{other}` is not implemented"),
    }
}
