use crate::command_helpers::prompt::persist_plan_output;
use crate::AppState;
use anyhow::{anyhow, bail, Context, Result};
use puffer_config::{ensure_workspace_dirs, ConfigPaths};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

pub(super) const DEFAULT_PLAN_TEXT: &str = "# Current Plan\n\n- Add concrete implementation steps here.\n";
const WORKFLOW_DIR_NAME: &str = "runtime/claude_workflow";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct StoredTask {
    pub(super) task_id: String,
    pub(super) subject: String,
    pub(super) description: String,
    pub(super) active_form: String,
    pub(super) status: String,
    pub(super) owner: Option<String>,
    pub(super) blocks: Vec<String>,
    pub(super) blocked_by: Vec<String>,
    pub(super) metadata: Map<String, Value>,
    pub(super) output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct StoredTodo {
    pub(super) content: String,
    pub(super) status: String,
    pub(super) active_form: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct StoredAgent {
    pub(super) agent_id: String,
    pub(super) name: Option<String>,
    pub(super) description: String,
    pub(super) prompt: String,
    pub(super) subagent_type: Option<String>,
    pub(super) model: Option<String>,
    pub(super) team_name: Option<String>,
    pub(super) mode: Option<String>,
    pub(super) isolation: Option<String>,
    pub(super) cwd: Option<String>,
    pub(super) status: String,
    pub(super) output_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct StoredTeam {
    pub(super) team_name: String,
    pub(super) description: Option<String>,
    pub(super) agent_type: Option<String>,
    pub(super) members: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct StoredCronJob {
    pub(super) id: String,
    pub(super) cron: String,
    pub(super) prompt: String,
    pub(super) recurring: bool,
    pub(super) durable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct StoredMessage {
    pub(super) id: String,
    pub(super) to: String,
    pub(super) summary: Option<String>,
    pub(super) message: Value,
    pub(super) created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct StoredWorktree {
    pub(super) name: String,
    pub(super) path: String,
    pub(super) base_cwd: String,
    pub(super) branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(super) struct TaskStore {
    pub(super) tasks: Vec<StoredTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(super) struct TodoStore {
    pub(super) todos: Vec<StoredTodo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(super) struct AgentStore {
    pub(super) agents: Vec<StoredAgent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(super) struct TeamStore {
    pub(super) teams: Vec<StoredTeam>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(super) struct CronStore {
    pub(super) jobs: Vec<StoredCronJob>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(super) struct MessageStore {
    pub(super) messages: Vec<StoredMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(super) struct WorktreeStore {
    pub(super) worktrees: Vec<StoredWorktree>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentInput {
    pub(super) description: String,
    pub(super) prompt: String,
    #[serde(default)]
    pub(super) subagent_type: Option<String>,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) run_in_background: bool,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) team_name: Option<String>,
    #[serde(default)]
    pub(super) mode: Option<String>,
    #[serde(default)]
    pub(super) isolation: Option<String>,
    #[serde(default)]
    pub(super) cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SendMessageInput {
    pub(super) to: String,
    #[serde(default)]
    pub(super) summary: Option<String>,
    pub(super) message: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct TeamCreateInput {
    pub(super) team_name: String,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) agent_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TodoWriteInput {
    pub(super) todos: Vec<TodoInputItem>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TodoInputItem {
    pub(super) content: String,
    pub(super) status: String,
    #[serde(rename = "activeForm")]
    pub(super) active_form: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TaskCreateInput {
    pub(super) subject: String,
    pub(super) description: String,
    #[serde(default, rename = "activeForm")]
    pub(super) active_form: Option<String>,
    #[serde(default)]
    pub(super) metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TaskIdInput {
    #[serde(rename = "taskId")]
    pub(super) task_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TaskUpdateInput {
    #[serde(rename = "taskId")]
    pub(super) task_id: String,
    #[serde(default)]
    pub(super) subject: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default, rename = "activeForm")]
    pub(super) active_form: Option<String>,
    #[serde(default)]
    pub(super) status: Option<String>,
    #[serde(default, rename = "addBlocks")]
    pub(super) add_blocks: Vec<String>,
    #[serde(default, rename = "addBlockedBy")]
    pub(super) add_blocked_by: Vec<String>,
    #[serde(default)]
    pub(super) owner: Option<String>,
    #[serde(default)]
    pub(super) metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TaskStopInput {
    #[serde(default)]
    pub(super) task_id: Option<String>,
    #[serde(default)]
    pub(super) shell_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TaskOutputInput {
    pub(super) task_id: String,
    #[serde(default)]
    pub(super) block: Option<bool>,
    #[serde(default)]
    pub(super) timeout: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ExitPlanModeInput {
    #[serde(default, rename = "allowedPrompts")]
    pub(super) allowed_prompts: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AskUserQuestionInput {
    pub(super) questions: Vec<Value>,
    #[serde(default)]
    pub(super) answers: Map<String, Value>,
    #[serde(default)]
    pub(super) annotations: Map<String, Value>,
    #[serde(default)]
    pub(super) metadata: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct EnterWorktreeInput {
    #[serde(default)]
    pub(super) name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ExitWorktreeInput {
    pub(super) action: String,
    #[serde(default)]
    pub(super) discard_changes: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConfigInput {
    pub(super) setting: String,
    #[serde(default)]
    pub(super) value: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspInput {
    pub(super) operation: String,
    #[serde(rename = "filePath")]
    pub(super) file_path: String,
    pub(super) line: usize,
    pub(super) character: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct PowerShellInput {
    pub(super) command: String,
    #[serde(default)]
    pub(super) timeout: Option<u64>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) run_in_background: bool,
    #[serde(default, rename = "dangerouslyDisableSandbox")]
    pub(super) dangerously_disable_sandbox: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct CronCreateInput {
    pub(super) cron: String,
    pub(super) prompt: String,
    #[serde(default)]
    pub(super) recurring: bool,
    #[serde(default)]
    pub(super) durable: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct CronDeleteInput {
    pub(super) id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SendUserMessageInput {
    pub(super) message: String,
    #[serde(default)]
    pub(super) attachments: Vec<String>,
    pub(super) status: String,
}

pub(super) fn append_agent_message(output_file: &Path, message: &Value) -> Result<()> {
    let previous = fs::read_to_string(output_file).unwrap_or_default();
    let next = format!(
        "{}\n\nMessage:\n{}\n",
        previous.trim_end(),
        serde_json::to_string_pretty(message)?
    );
    fs::write(output_file, next)
        .with_context(|| format!("failed to write {}", output_file.display()))
}

pub(super) fn resolve_recipients(cwd: &Path, target: &str) -> Result<Vec<String>> {
    if target == "*" {
        let agents = load_store::<AgentStore>(&agents_path(cwd))?;
        return Ok(agents.agents.into_iter().map(|agent| agent.agent_id).collect());
    }

    let teams = load_store::<TeamStore>(&teams_path(cwd))?;
    if let Some(team) = teams.teams.iter().find(|team| team.team_name == target) {
        return Ok(team.members.clone());
    }

    let agents = load_store::<AgentStore>(&agents_path(cwd))?;
    if agents
        .agents
        .iter()
        .any(|agent| agent.agent_id == target || agent.name.as_deref() == Some(target))
    {
        return Ok(vec![target.to_string()]);
    }

    Ok(Vec::new())
}

pub(super) fn get_config_value(state: &AppState, setting: &str) -> Result<Value> {
    match setting {
        "theme" => Ok(json!(state.config.theme)),
        "default_provider" => Ok(json!(state.config.default_provider)),
        "default_model" => Ok(json!(state.config.default_model)),
        "openai_base_url" => Ok(json!(state.config.openai_base_url)),
        "no_alt_screen" => Ok(json!(state.config.ui.no_alt_screen)),
        "tmux_golden_mode" => Ok(json!(state.config.ui.tmux_golden_mode)),
        other => bail!("Unsupported config setting `{other}`"),
    }
}

pub(super) fn set_config_value(state: &mut AppState, setting: &str, value: Value) -> Result<()> {
    match setting {
        "theme" => {
            state.config.theme = value
                .as_str()
                .ok_or_else(|| anyhow!("theme must be a string"))?
                .to_string()
        }
        "default_provider" => {
            state.config.default_provider = match value {
                Value::Null => None,
                Value::String(text) => Some(text),
                _ => bail!("default_provider must be a string"),
            }
        }
        "default_model" => {
            state.config.default_model = match value {
                Value::Null => None,
                Value::String(text) => Some(text),
                _ => bail!("default_model must be a string"),
            }
        }
        "openai_base_url" => {
            state.config.openai_base_url = match value {
                Value::Null => None,
                Value::String(text) => Some(text),
                _ => bail!("openai_base_url must be a string"),
            }
        }
        "no_alt_screen" => {
            state.config.ui.no_alt_screen = value
                .as_bool()
                .ok_or_else(|| anyhow!("no_alt_screen must be a boolean"))?
        }
        "tmux_golden_mode" => {
            state.config.ui.tmux_golden_mode = value
                .as_bool()
                .ok_or_else(|| anyhow!("tmux_golden_mode must be a boolean"))?
        }
        other => bail!("Unsupported config setting `{other}`"),
    }
    Ok(())
}

pub(super) fn detect_powershell_binary() -> Result<String> {
    for candidate in ["pwsh", "powershell"] {
        if Command::new(candidate)
            .arg("-NoLogo")
            .arg("-Command")
            .arg("$PSVersionTable.PSVersion.ToString()")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Ok(candidate.to_string());
        }
    }
    bail!("PowerShell is not installed on this system")
}

pub(super) fn identifier_at_position(content: &str, line: usize, character: usize) -> Option<String> {
    let line_text = content.lines().nth(line.saturating_sub(1))?;
    if line_text.is_empty() {
        return None;
    }
    let chars = line_text.chars().collect::<Vec<_>>();
    let mut index = character.saturating_sub(1).min(chars.len().saturating_sub(1));
    if !is_identifier_char(chars[index]) && index > 0 {
        index -= 1;
    }
    if !is_identifier_char(chars[index]) {
        return None;
    }
    let mut start = index;
    while start > 0 && is_identifier_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < chars.len() && is_identifier_char(chars[end]) {
        end += 1;
    }
    Some(chars[start..end].iter().collect())
}

pub(super) fn line_text(content: &str, line: usize) -> String {
    content
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or_default()
        .to_string()
}

pub(super) fn document_symbols(content: &str) -> Vec<Value> {
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| detect_symbol_name(line).map(|name| (index + 1, name)))
        .map(|(line, name)| {
            json!({
                "name": name,
                "line": line
            })
        })
        .collect()
}

pub(super) fn workspace_symbols(cwd: &Path) -> Result<Vec<Value>> {
    let mut files = Vec::new();
    collect_workspace_files(cwd, cwd, &mut files)?;
    let mut symbols = Vec::new();
    for file in files {
        let content = match fs::read_to_string(&file) {
            Ok(content) => content,
            Err(_) => continue,
        };
        for symbol in document_symbols(&content) {
            symbols.push(json!({
                "filePath": file.display().to_string(),
                "symbol": symbol
            }));
        }
    }
    Ok(symbols)
}

pub(super) fn search_workspace_identifier(cwd: &Path, identifier: &str) -> Result<Vec<Value>> {
    let mut files = Vec::new();
    collect_workspace_files(cwd, cwd, &mut files)?;
    let mut matches = Vec::new();
    for file in files {
        let content = match fs::read_to_string(&file) {
            Ok(content) => content,
            Err(_) => continue,
        };
        for (index, line) in content.lines().enumerate() {
            if let Some(character) = line.find(identifier) {
                matches.push(json!({
                    "filePath": file.display().to_string(),
                    "line": index + 1,
                    "character": character + 1,
                    "text": line.trim()
                }));
                if matches.len() >= 50 {
                    return Ok(matches);
                }
            }
        }
    }
    Ok(matches)
}

pub(super) fn load_store<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    if !path.exists() {
        return Ok(T::default());
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read workflow store {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse workflow store {}", path.display()))
}

pub(super) fn save_store<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(value)?;
    fs::write(path, raw).with_context(|| format!("failed to write {}", path.display()))
}

pub(super) fn workflow_root(cwd: &Path) -> Result<PathBuf> {
    let paths = ConfigPaths::discover(cwd);
    ensure_workspace_dirs(&paths)?;
    let root = paths.workspace_config_dir.join(WORKFLOW_DIR_NAME);
    fs::create_dir_all(&root)?;
    Ok(root)
}

pub(super) fn tasks_path(cwd: &Path) -> PathBuf {
    workflow_root(cwd).unwrap().join("tasks.json")
}

pub(super) fn todos_path(cwd: &Path) -> PathBuf {
    workflow_root(cwd).unwrap().join("todos.json")
}

pub(super) fn agents_path(cwd: &Path) -> PathBuf {
    workflow_root(cwd).unwrap().join("agents.json")
}

pub(super) fn teams_path(cwd: &Path) -> PathBuf {
    workflow_root(cwd).unwrap().join("teams.json")
}

pub(super) fn crons_path(cwd: &Path) -> PathBuf {
    workflow_root(cwd).unwrap().join("crons.json")
}

pub(super) fn messages_path(cwd: &Path) -> PathBuf {
    workflow_root(cwd).unwrap().join("messages.json")
}

pub(super) fn worktrees_path(cwd: &Path) -> PathBuf {
    workflow_root(cwd).unwrap().join("worktrees.json")
}

pub(super) fn ensure_plan_file(state: &AppState) -> Result<PathBuf> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let plan_dir = paths.workspace_config_dir.join("plans");
    fs::create_dir_all(&plan_dir)?;
    let plan_path = plan_dir.join(format!("{}.md", state.session.id));
    if !plan_path.exists() {
        persist_plan_output(state, DEFAULT_PLAN_TEXT)?;
    }
    Ok(plan_path)
}

pub(super) fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["-C", cwd.to_string_lossy().as_ref(), "rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

pub(super) fn is_git_repo(path: &Path) -> bool {
    Command::new("git")
        .args(["-C", path.to_string_lossy().as_ref(), "rev-parse", "--git-dir"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(super) fn git_dirty(path: &Path) -> Result<bool> {
    let output = Command::new("git")
        .args(["-C", path.to_string_lossy().as_ref(), "status", "--porcelain"])
        .output()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
}

pub(super) fn resolve_path(cwd: &Path, path: &str) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        cwd.join(candidate)
    }
}

pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn collect_workspace_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let relative_text = relative.to_string_lossy();
        if relative_text.starts_with(".git")
            || relative_text.starts_with("target")
            || relative_text.starts_with(".worktree")
            || relative_text.starts_with(".puffer/runtime/claude_workflow")
        {
            continue;
        }
        if file_type.is_dir() {
            collect_workspace_files(root, &path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn detect_symbol_name(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    for prefix in [
        "pub fn ",
        "fn ",
        "pub struct ",
        "struct ",
        "pub enum ",
        "enum ",
        "pub trait ",
        "trait ",
        "class ",
        "function ",
        "impl ",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let name = rest
                .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                .find(|part| !part.is_empty())?;
            return Some(name.to_string());
        }
    }
    None
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
