use crate::AppState;
use anyhow::{anyhow, bail, Context, Result};
use puffer_config::{ensure_workspace_dirs, save_workspace_config, ConfigPaths};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use uuid::Uuid;
use super::store::{
    agents_path, append_agent_message, crons_path, detect_powershell_binary, document_symbols,
    ensure_plan_file, get_config_value, git_dirty, git_toplevel, identifier_at_position,
    is_git_repo, line_text, load_store, messages_path, now_ms, resolve_path, resolve_recipients,
    save_store, search_workspace_identifier, set_config_value, tasks_path, teams_path,
    todos_path, workflow_root, workspace_symbols, worktrees_path, AgentInput, AgentStore,
    AskUserQuestionInput, ConfigInput, CronCreateInput, CronDeleteInput, CronStore,
    EnterWorktreeInput, ExitPlanModeInput, ExitWorktreeInput, LspInput, MessageStore,
    PowerShellInput, SendMessageInput, SendUserMessageInput, StoredAgent, StoredCronJob,
    StoredMessage, StoredTask, StoredTeam, StoredTodo, StoredWorktree, TaskCreateInput,
    TaskIdInput, TaskOutputInput, TaskStopInput, TaskStore, TaskUpdateInput, TeamCreateInput,
    TeamStore, TodoStore, TodoWriteInput, WorktreeStore,
};

/// Executes the live `Agent` workflow tool.
pub(super) fn execute_agent(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    let parsed: AgentInput = serde_json::from_value(input).context("invalid Agent input")?;
    let agent_id = format!("agent-{}", Uuid::new_v4().simple());
    let store_cwd = state.session.cwd.as_path();
    let root = workflow_root(store_cwd)?;
    let output_dir = root.join("agent_outputs");
    fs::create_dir_all(&output_dir)?;
    let output_file = output_dir.join(format!("{agent_id}.md"));
    let output_text = format!(
        "# Agent {}\n\nDescription: {}\n\nPrompt:\n{}\n",
        agent_id, parsed.description, parsed.prompt
    );
    fs::write(&output_file, output_text)?;

    let mut agents = load_store::<AgentStore>(&agents_path(store_cwd))?;
    let agent = StoredAgent {
        agent_id: agent_id.clone(),
        name: parsed.name.clone(),
        description: parsed.description.clone(),
        prompt: parsed.prompt.clone(),
        subagent_type: parsed.subagent_type.clone(),
        model: parsed.model.clone(),
        team_name: parsed.team_name.clone(),
        mode: parsed.mode.clone(),
        isolation: parsed.isolation.clone(),
        cwd: parsed.cwd.clone(),
        status: if parsed.run_in_background {
            "async_launched".to_string()
        } else {
            "completed".to_string()
        },
        output_file: output_file.display().to_string(),
    };
    agents.agents.push(agent.clone());
    save_store(&agents_path(store_cwd), &agents)?;

    if let Some(team_name) = parsed.team_name.as_deref() {
        let mut teams = load_store::<TeamStore>(&teams_path(store_cwd))?;
        if let Some(team) = teams.teams.iter_mut().find(|team| team.team_name == team_name) {
            if !team.members.iter().any(|member| member == &agent_id) {
                team.members.push(agent_id.clone());
            }
            save_store(&teams_path(store_cwd), &teams)?;
        }
    }

    state.record_task("Agent", parsed.description.clone(), true);
    Ok(serde_json::to_string_pretty(&json!({
        "status": agent.status,
        "agentId": agent.agent_id,
        "description": agent.description,
        "prompt": agent.prompt,
        "outputFile": agent.output_file,
        "canReadOutputFile": true
    }))?)
}

/// Executes the live `SendMessage` workflow tool.
pub(super) fn execute_send_message(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: SendMessageInput =
        serde_json::from_value(input).context("invalid SendMessage input")?;
    let store_cwd = state.session.cwd.as_path();
    let recipients = resolve_recipients(store_cwd, &parsed.to)?;
    if recipients.is_empty() {
        bail!("SendMessage could not resolve recipient `{}`", parsed.to);
    }

    let mut messages = load_store::<MessageStore>(&messages_path(store_cwd))?;
    let mut agents = load_store::<AgentStore>(&agents_path(store_cwd))?;
    for recipient in &recipients {
        messages.messages.push(StoredMessage {
            id: format!("msg-{}", Uuid::new_v4().simple()),
            to: recipient.clone(),
            summary: parsed.summary.clone(),
            message: parsed.message.clone(),
            created_at_ms: now_ms(),
        });
        if let Some(agent) = agents.agents.iter_mut().find(|agent| {
            agent.agent_id == *recipient || agent.name.as_deref() == Some(recipient.as_str())
        }) {
            append_agent_message(Path::new(&agent.output_file), &parsed.message)?;
            if parsed
                .message
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "shutdown_request")
            {
                agent.status = "stopped".to_string();
            }
        }
    }
    save_store(&messages_path(store_cwd), &messages)?;
    save_store(&agents_path(store_cwd), &agents)?;
    Ok(serde_json::to_string_pretty(&json!({
        "delivered": recipients,
        "summary": parsed.summary,
        "message": parsed.message
    }))?)
}

/// Executes the live `TeamCreate` workflow tool.
pub(super) fn execute_team_create(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: TeamCreateInput = serde_json::from_value(input).context("invalid TeamCreate input")?;
    let mut teams = load_store::<TeamStore>(&teams_path(state.session.cwd.as_path()))?;
    if teams.teams.iter().any(|team| team.team_name == parsed.team_name) {
        bail!("team `{}` already exists", parsed.team_name);
    }
    teams.teams.push(StoredTeam {
        team_name: parsed.team_name.clone(),
        description: parsed.description.clone(),
        agent_type: parsed.agent_type.clone(),
        members: Vec::new(),
    });
    save_store(&teams_path(state.session.cwd.as_path()), &teams)?;
    Ok(serde_json::to_string_pretty(&json!({
        "team_name": parsed.team_name,
        "description": parsed.description,
        "agent_type": parsed.agent_type,
        "members": []
    }))?)
}

/// Executes the live `TeamDelete` workflow tool.
pub(super) fn execute_team_delete(
    state: &mut AppState,
    cwd: &Path,
    _input: Value,
) -> Result<String> {
    let mut teams = load_store::<TeamStore>(&teams_path(state.session.cwd.as_path()))?;
    let deleted = teams.teams.drain(..).map(|team| team.team_name).collect::<Vec<_>>();
    save_store(&teams_path(state.session.cwd.as_path()), &teams)?;
    Ok(serde_json::to_string_pretty(&json!({
        "deleted": deleted
    }))?)
}

/// Executes the live `TodoWrite` workflow tool.
pub(super) fn execute_todo_write(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: TodoWriteInput = serde_json::from_value(input).context("invalid TodoWrite input")?;
    let mut store = load_store::<TodoStore>(&todos_path(state.session.cwd.as_path()))?;
    let old = store.todos.clone();
    store.todos = parsed
        .todos
        .into_iter()
        .map(|todo| StoredTodo {
            content: todo.content,
            status: todo.status,
            active_form: todo.active_form,
        })
        .collect();
    save_store(&todos_path(state.session.cwd.as_path()), &store)?;
    Ok(serde_json::to_string_pretty(&json!({
        "oldTodos": old,
        "newTodos": store.todos
    }))?)
}

/// Executes the live `TaskCreate` workflow tool.
pub(super) fn execute_task_create(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: TaskCreateInput = serde_json::from_value(input).context("invalid TaskCreate input")?;
    let mut store = load_store::<TaskStore>(&tasks_path(state.session.cwd.as_path()))?;
    let task = StoredTask {
        task_id: format!("task-{}", Uuid::new_v4().simple()),
        subject: parsed.subject,
        description: parsed.description,
        active_form: parsed.active_form.unwrap_or_else(|| "Working".to_string()),
        status: "pending".to_string(),
        owner: None,
        blocks: Vec::new(),
        blocked_by: Vec::new(),
        metadata: parsed.metadata.unwrap_or_default(),
        output: None,
    };
    store.tasks.push(task.clone());
    save_store(&tasks_path(state.session.cwd.as_path()), &store)?;
    Ok(serde_json::to_string_pretty(&task)?)
}

/// Executes the live `TaskGet` workflow tool.
pub(super) fn execute_task_get(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: TaskIdInput = serde_json::from_value(input).context("invalid TaskGet input")?;
    let store = load_store::<TaskStore>(&tasks_path(state.session.cwd.as_path()))?;
    let task = store
        .tasks
        .into_iter()
        .find(|task| task.task_id == parsed.task_id)
        .ok_or_else(|| anyhow!("unknown task `{}`", parsed.task_id))?;
    Ok(serde_json::to_string_pretty(&task)?)
}

/// Executes the live `TaskList` workflow tool.
pub(super) fn execute_task_list(
    state: &mut AppState,
    cwd: &Path,
    _input: Value,
) -> Result<String> {
    let store = load_store::<TaskStore>(&tasks_path(state.session.cwd.as_path()))?;
    Ok(serde_json::to_string_pretty(&store.tasks)?)
}

/// Executes the live `TaskUpdate` workflow tool.
pub(super) fn execute_task_update(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: TaskUpdateInput = serde_json::from_value(input).context("invalid TaskUpdate input")?;
    let mut store = load_store::<TaskStore>(&tasks_path(state.session.cwd.as_path()))?;
    let task = store
        .tasks
        .iter_mut()
        .find(|task| task.task_id == parsed.task_id)
        .ok_or_else(|| anyhow!("unknown task `{}`", parsed.task_id))?;
    if let Some(subject) = parsed.subject {
        task.subject = subject;
    }
    if let Some(description) = parsed.description {
        task.description = description;
    }
    if let Some(active_form) = parsed.active_form {
        task.active_form = active_form;
    }
    if let Some(status) = parsed.status {
        task.status = status;
    }
    if let Some(owner) = parsed.owner {
        task.owner = Some(owner);
    }
    for block in parsed.add_blocks {
        if !task.blocks.iter().any(|existing| existing == &block) {
            task.blocks.push(block);
        }
    }
    for blocked_by in parsed.add_blocked_by {
        if !task.blocked_by.iter().any(|existing| existing == &blocked_by) {
            task.blocked_by.push(blocked_by);
        }
    }
    if let Some(metadata) = parsed.metadata {
        for (key, value) in metadata {
            if value.is_null() {
                task.metadata.remove(&key);
            } else {
                task.metadata.insert(key, value);
            }
        }
    }
    let output = task.clone();
    save_store(&tasks_path(state.session.cwd.as_path()), &store)?;
    Ok(serde_json::to_string_pretty(&output)?)
}

/// Executes the live `TaskStop` workflow tool.
pub(super) fn execute_task_stop(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: TaskStopInput = serde_json::from_value(input).context("invalid TaskStop input")?;
    let target = parsed
        .task_id
        .or(parsed.shell_id)
        .ok_or_else(|| anyhow!("TaskStop requires task_id or shell_id"))?;

    let store_cwd = state.session.cwd.as_path();
    let mut tasks = load_store::<TaskStore>(&tasks_path(store_cwd))?;
    if let Some(task) = tasks.tasks.iter_mut().find(|task| task.task_id == target) {
        task.status = "completed".to_string();
        task.output = Some("Stopped by TaskStop.".to_string());
        let output = task.clone();
        save_store(&tasks_path(store_cwd), &tasks)?;
        return Ok(serde_json::to_string_pretty(&output)?);
    }

    let mut agents = load_store::<AgentStore>(&agents_path(store_cwd))?;
    if let Some(agent) = agents.agents.iter_mut().find(|agent| agent.agent_id == target) {
        agent.status = "stopped".to_string();
        append_agent_message(Path::new(&agent.output_file), &json!("Stopped by TaskStop."))?;
        let output = json!({
            "task_id": target,
            "status": agent.status,
            "output_file": agent.output_file
        });
        save_store(&agents_path(store_cwd), &agents)?;
        return Ok(serde_json::to_string_pretty(&output)?);
    }

    bail!("unknown task `{}`", target)
}

/// Executes the live `TaskOutput` workflow tool.
pub(super) fn execute_task_output(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: TaskOutputInput = serde_json::from_value(input).context("invalid TaskOutput input")?;
    let store_cwd = state.session.cwd.as_path();
    let tasks = load_store::<TaskStore>(&tasks_path(store_cwd))?;
    if let Some(task) = tasks.tasks.iter().find(|task| task.task_id == parsed.task_id) {
        return Ok(serde_json::to_string_pretty(&json!({
            "task_id": task.task_id,
            "status": task.status,
            "output": task.output,
            "block": parsed.block.unwrap_or(true),
            "timeout": parsed.timeout.unwrap_or(30_000)
        }))?);
    }
    let agents = load_store::<AgentStore>(&agents_path(store_cwd))?;
    if let Some(agent) = agents.agents.iter().find(|agent| agent.agent_id == parsed.task_id) {
        let output = fs::read_to_string(&agent.output_file).unwrap_or_default();
        return Ok(serde_json::to_string_pretty(&json!({
            "task_id": agent.agent_id,
            "status": agent.status,
            "output": output,
            "outputFile": agent.output_file,
            "block": parsed.block.unwrap_or(true),
            "timeout": parsed.timeout.unwrap_or(30_000)
        }))?);
    }
    bail!("unknown task `{}`", parsed.task_id)
}

/// Executes the live `EnterPlanMode` workflow tool.
pub(super) fn execute_enter_plan_mode(
    state: &mut AppState,
    _cwd: &Path,
    _input: Value,
) -> Result<String> {
    state.plan_mode = true;
    let plan_path = ensure_plan_file(state)?;
    Ok(serde_json::to_string_pretty(&json!({
        "status": "entered",
        "planMode": true,
        "planFile": plan_path.display().to_string(),
        "plan": fs::read_to_string(plan_path).unwrap_or_default()
    }))?)
}

/// Executes the live `ExitPlanMode` workflow tool.
pub(super) fn execute_exit_plan_mode(
    state: &mut AppState,
    _cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: ExitPlanModeInput =
        serde_json::from_value(input).context("invalid ExitPlanMode input")?;
    state.plan_mode = false;
    let plan_path = ensure_plan_file(state)?;
    Ok(serde_json::to_string_pretty(&json!({
        "status": "exited",
        "planMode": false,
        "planFile": plan_path.display().to_string(),
        "allowedPrompts": parsed.allowed_prompts
    }))?)
}

/// Executes the live `AskUserQuestion` workflow tool.
pub(super) fn execute_ask_user_question(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: AskUserQuestionInput =
        serde_json::from_value(input).context("invalid AskUserQuestion input")?;
    let pending_path = workflow_root(state.session.cwd.as_path())?.join("pending_questions.json");
    fs::write(&pending_path, serde_json::to_string_pretty(&parsed.questions)?)?;
    Ok(serde_json::to_string_pretty(&json!({
        "questions": parsed.questions,
        "answers": parsed.answers,
        "annotations": parsed.annotations,
        "metadata": parsed.metadata,
        "pending": parsed.answers.is_empty(),
        "pendingFile": pending_path.display().to_string()
    }))?)
}

/// Executes the live `EnterWorktree` workflow tool.
pub(super) fn execute_enter_worktree(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: EnterWorktreeInput =
        serde_json::from_value(input).context("invalid EnterWorktree input")?;
    let worktree_name = parsed
        .name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("worktree-{}", Uuid::new_v4().simple()));
    let base_cwd = cwd.to_path_buf();
    let repo_root = git_toplevel(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let worktree_root = repo_root.join(".worktree");
    fs::create_dir_all(&worktree_root)?;
    let worktree_path = worktree_root.join(&worktree_name);
    let mut branch = None;
    if is_git_repo(&repo_root) {
        let branch_name = format!("puffer-{}", Uuid::new_v4().simple());
        let status = Command::new("git")
            .args([
                "-C",
                repo_root.to_string_lossy().as_ref(),
                "worktree",
                "add",
                "-b",
                &branch_name,
                worktree_path.to_string_lossy().as_ref(),
            ])
            .status()
            .context("failed to launch git worktree add")?;
        if !status.success() {
            bail!("git worktree add failed for {}", worktree_path.display());
        }
        branch = Some(branch_name);
    } else {
        fs::create_dir_all(&worktree_path)?;
    }

    let mut store = load_store::<WorktreeStore>(&worktrees_path(state.session.cwd.as_path()))?;
    store.worktrees.push(StoredWorktree {
        name: worktree_name.clone(),
        path: worktree_path.display().to_string(),
        base_cwd: base_cwd.display().to_string(),
        branch: branch.clone(),
    });
    save_store(&worktrees_path(state.session.cwd.as_path()), &store)?;

    if !state.working_dirs.iter().any(|path| path == &worktree_path) {
        state.working_dirs.push(worktree_path.clone());
    }
    state.cwd = worktree_path.clone();
    Ok(serde_json::to_string_pretty(&json!({
        "name": worktree_name,
        "path": worktree_path.display().to_string(),
        "branch": branch,
        "baseCwd": base_cwd.display().to_string()
    }))?)
}

/// Executes the live `ExitWorktree` workflow tool.
pub(super) fn execute_exit_worktree(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: ExitWorktreeInput =
        serde_json::from_value(input).context("invalid ExitWorktree input")?;
    let mut store = load_store::<WorktreeStore>(&worktrees_path(state.session.cwd.as_path()))?;
    let index = store
        .worktrees
        .iter()
        .position(|worktree| Path::new(&worktree.path) == cwd)
        .or_else(|| store.worktrees.len().checked_sub(1))
        .ok_or_else(|| anyhow!("no active worktree session found"))?;
    let entry = store.worktrees[index].clone();
    let worktree_path = PathBuf::from(&entry.path);
    let base_cwd = PathBuf::from(&entry.base_cwd);

    if parsed.action == "remove" {
        if is_git_repo(&base_cwd) {
            let dirty = git_dirty(&worktree_path).unwrap_or(false);
            if dirty && !parsed.discard_changes {
                bail!("worktree has uncommitted changes; set discard_changes=true to remove it");
            }
            let mut args = vec![
                "-C".to_string(),
                base_cwd.to_string_lossy().to_string(),
                "worktree".to_string(),
                "remove".to_string(),
            ];
            if parsed.discard_changes {
                args.push("--force".to_string());
            }
            args.push(worktree_path.to_string_lossy().to_string());
            let status = Command::new("git")
                .args(args)
                .status()
                .context("failed to launch git worktree remove")?;
            if !status.success() {
                bail!("git worktree remove failed for {}", worktree_path.display());
            }
        } else if worktree_path.exists() {
            fs::remove_dir_all(&worktree_path)
                .with_context(|| format!("failed to remove {}", worktree_path.display()))?;
        }
        store.worktrees.remove(index);
    }
    save_store(&worktrees_path(state.session.cwd.as_path()), &store)?;
    state.cwd = base_cwd.clone();
    state.working_dirs.retain(|path| path != &worktree_path);
    Ok(serde_json::to_string_pretty(&json!({
        "action": parsed.action,
        "returnedTo": base_cwd.display().to_string(),
        "worktreePath": worktree_path.display().to_string()
    }))?)
}

/// Executes the live `Config` workflow tool.
pub(super) fn execute_config(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: ConfigInput = serde_json::from_value(input).context("invalid Config input")?;
    let paths = ConfigPaths::discover(cwd);
    ensure_workspace_dirs(&paths)?;
    if let Some(value) = parsed.value {
        set_config_value(state, &parsed.setting, value)?;
        save_workspace_config(&paths, &state.config)?;
    }
    let current = get_config_value(state, &parsed.setting)?;
    Ok(serde_json::to_string_pretty(&json!({
        "setting": parsed.setting,
        "value": current,
        "path": paths.workspace_config_file().display().to_string()
    }))?)
}

/// Executes the live `LSP` workflow tool.
pub(super) fn execute_lsp(
    _state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: LspInput = serde_json::from_value(input).context("invalid LSP input")?;
    let file_path = resolve_path(cwd, &parsed.file_path);
    let content = fs::read_to_string(&file_path)
        .with_context(|| format!("failed to read {}", file_path.display()))?;
    let identifier = identifier_at_position(&content, parsed.line, parsed.character);
    let output = match parsed.operation.as_str() {
        "hover" => json!({
            "operation": parsed.operation,
            "filePath": file_path.display().to_string(),
            "line": parsed.line,
            "character": parsed.character,
            "identifier": identifier,
            "lineText": line_text(&content, parsed.line),
        }),
        "diagnostics" => json!({
            "operation": parsed.operation,
            "filePath": file_path.display().to_string(),
            "items": [],
        }),
        "documentSymbol" => json!({
            "operation": parsed.operation,
            "filePath": file_path.display().to_string(),
            "symbols": document_symbols(&content),
        }),
        "workspaceSymbol" => json!({
            "operation": parsed.operation,
            "symbols": workspace_symbols(cwd)?,
        }),
        "goToDefinition" | "findReferences" | "goToImplementation" | "prepareCallHierarchy"
        | "incomingCalls" | "outgoingCalls" => json!({
            "operation": parsed.operation,
            "filePath": file_path.display().to_string(),
            "identifier": identifier,
            "matches": identifier
                .as_deref()
                .map(|value| search_workspace_identifier(cwd, value))
                .transpose()?
                .unwrap_or_default(),
        }),
        other => bail!("unsupported LSP operation `{other}`"),
    };
    Ok(serde_json::to_string_pretty(&output)?)
}

/// Executes the live `PowerShell` workflow tool.
pub(super) fn execute_powershell(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: PowerShellInput =
        serde_json::from_value(input).context("invalid PowerShell input")?;
    let shell = detect_powershell_binary()?;
    if parsed.run_in_background {
        let child = Command::new(&shell)
            .args(["-NoLogo", "-Command", &parsed.command])
            .current_dir(cwd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start {}", shell))?;
        let task_id = format!("shell-{}", child.id());
        state.record_task("PowerShell", parsed.command.clone(), true);
        return Ok(serde_json::to_string_pretty(&json!({
            "stdout": "",
            "stderr": "",
            "interrupted": false,
            "backgroundTaskId": task_id,
            "dangerouslyDisableSandbox": parsed.dangerously_disable_sandbox
        }))?);
    }

    let timeout_ms = parsed.timeout.unwrap_or(120_000).clamp(1, 600_000);
    let output = Command::new(&shell)
        .args(["-NoLogo", "-Command", &parsed.command])
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to execute {}", shell))?;
    state.record_task(
        parsed
            .description
            .clone()
            .unwrap_or_else(|| "PowerShell".to_string()),
        parsed.command.clone(),
        output.status.success(),
    );
    Ok(serde_json::to_string_pretty(&json!({
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
        "interrupted": false,
        "dangerouslyDisableSandbox": parsed.dangerously_disable_sandbox,
        "timeoutMs": timeout_ms
    }))?)
}

/// Executes the live `CronCreate` workflow tool.
pub(super) fn execute_cron_create(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: CronCreateInput =
        serde_json::from_value(input).context("invalid CronCreate input")?;
    let mut store = load_store::<CronStore>(&crons_path(state.session.cwd.as_path()))?;
    let job = StoredCronJob {
        id: format!("cron-{}", Uuid::new_v4().simple()),
        cron: parsed.cron,
        prompt: parsed.prompt,
        recurring: parsed.recurring,
        durable: parsed.durable,
    };
    store.jobs.push(job.clone());
    save_store(&crons_path(state.session.cwd.as_path()), &store)?;
    Ok(serde_json::to_string_pretty(&job)?)
}

/// Executes the live `CronList` workflow tool.
pub(super) fn execute_cron_list(
    state: &mut AppState,
    cwd: &Path,
    _input: Value,
) -> Result<String> {
    let store = load_store::<CronStore>(&crons_path(state.session.cwd.as_path()))?;
    Ok(serde_json::to_string_pretty(&store.jobs)?)
}

/// Executes the live `CronDelete` workflow tool.
pub(super) fn execute_cron_delete(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: CronDeleteInput =
        serde_json::from_value(input).context("invalid CronDelete input")?;
    let mut store = load_store::<CronStore>(&crons_path(state.session.cwd.as_path()))?;
    let before = store.jobs.len();
    store.jobs.retain(|job| job.id != parsed.id);
    save_store(&crons_path(state.session.cwd.as_path()), &store)?;
    Ok(serde_json::to_string_pretty(&json!({
        "deleted": before != store.jobs.len(),
        "id": parsed.id
    }))?)
}

/// Executes the live `SendUserMessage` or `Brief` workflow tool.
pub(super) fn execute_send_user_message(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: SendUserMessageInput =
        serde_json::from_value(input).context("invalid SendUserMessage input")?;
    let mut messages = load_store::<MessageStore>(&messages_path(state.session.cwd.as_path()))?;
    messages.messages.push(StoredMessage {
        id: format!("user-msg-{}", Uuid::new_v4().simple()),
        to: "user".to_string(),
        summary: Some(parsed.status.clone()),
        message: json!({
            "message": parsed.message,
            "attachments": parsed.attachments,
            "status": parsed.status,
        }),
        created_at_ms: now_ms(),
    });
    save_store(&messages_path(state.session.cwd.as_path()), &messages)?;
    Ok(serde_json::to_string_pretty(&messages.messages.last().cloned())?)
}

/// Executes the live `StructuredOutput` workflow tool.
pub(super) fn execute_structured_output(
    _state: &mut AppState,
    _cwd: &Path,
    input: Value,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&input)?)
}
