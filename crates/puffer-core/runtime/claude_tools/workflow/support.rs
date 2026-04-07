use super::store::{
    agents_path, append_agent_message, detect_powershell_binary, document_symbols,
    ensure_plan_file, get_config_value, git_ahead_count, git_dirty, git_head_commit, git_toplevel,
    identifier_at_position, is_git_repo, line_text, load_store, messages_path, next_task_id,
    now_ms, process_is_running, resolve_path, resolve_recipients, save_store,
    search_workspace_identifier, set_config_value, task_output_path, tasks_path, teams_path,
    terminate_process, todos_path, validate_ask_user_questions, wait_for_process_exit,
    workflow_root, workspace_symbols, worktrees_path, AgentInput, AgentStore, AskUserQuestionInput,
    ConfigInput, EnterWorktreeInput, ExitPlanModeInput, ExitWorktreeInput, LspInput, MessageStore,
    PowerShellInput, SendMessageInput, StoredAgent, StoredMessage, StoredTask, StoredTeam,
    StoredTodo, StoredWorktree, TaskCreateInput, TaskIdInput, TaskOutputInput, TaskStopInput,
    TaskStore, TaskUpdateInput, TeamCreateInput, TeamStore, TodoStore, TodoWriteInput,
    WorktreeStore,
};
use super::task_runtime::{
    read_task_output, refresh_stored_task, terminal_task_status, wait_for_child_output,
    wait_for_stored_task,
};
use crate::AppState;
use anyhow::{anyhow, bail, Context, Result};
use puffer_config::{ensure_workspace_dirs, save_workspace_config, ConfigPaths};
use serde_json::{json, Value};
use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use uuid::Uuid;

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
        if let Some(team) = teams
            .teams
            .iter_mut()
            .find(|team| team.team_name == team_name)
        {
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
    let parsed: TeamCreateInput =
        serde_json::from_value(input).context("invalid TeamCreate input")?;
    let mut teams = load_store::<TeamStore>(&teams_path(state.session.cwd.as_path()))?;
    if teams
        .teams
        .iter()
        .any(|team| team.team_name == parsed.team_name)
    {
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
    let agents = load_store::<AgentStore>(&agents_path(state.session.cwd.as_path()))?;
    let teams_with_active_members = teams
        .teams
        .iter()
        .filter(|team| {
            team.members.iter().any(|member| {
                agents
                    .agents
                    .iter()
                    .find(|agent| &agent.agent_id == member)
                    .is_some_and(|agent| !terminal_task_status(&agent.status))
            })
        })
        .map(|team| team.team_name.clone())
        .collect::<Vec<_>>();
    if !teams_with_active_members.is_empty() {
        bail!(
            "cannot delete teams with active members: {}",
            teams_with_active_members.join(", ")
        );
    }
    let deleted = teams
        .teams
        .drain(..)
        .map(|team| team.team_name)
        .collect::<Vec<_>>();
    save_store(&teams_path(state.session.cwd.as_path()), &teams)?;
    Ok(serde_json::to_string_pretty(&json!({
        "deleted": deleted
    }))?)
}

/// Executes the live `TodoWrite` workflow tool.
pub(super) fn execute_todo_write(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    let parsed: TodoWriteInput =
        serde_json::from_value(input).context("invalid TodoWrite input")?;
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
    let parsed: TaskCreateInput =
        serde_json::from_value(input).context("invalid TaskCreate input")?;
    let mut store = load_store::<TaskStore>(&tasks_path(state.session.cwd.as_path()))?;
    let task = StoredTask {
        task_id: next_task_id(&store.tasks),
        subject: parsed.subject,
        description: parsed.description,
        active_form: parsed.active_form.unwrap_or_else(|| "Working".to_string()),
        status: "pending".to_string(),
        owner: None,
        blocks: Vec::new(),
        blocked_by: Vec::new(),
        metadata: parsed.metadata.unwrap_or_default(),
        output: None,
        task_type: Some("task".to_string()),
        command: None,
        process_id: None,
        output_file: None,
        started_at_ms: Some(now_ms()),
        updated_at_ms: Some(now_ms()),
        exit_code: None,
    };
    store.tasks.push(task.clone());
    save_store(&tasks_path(state.session.cwd.as_path()), &store)?;
    Ok(serde_json::to_string_pretty(&task)?)
}

/// Executes the live `TaskGet` workflow tool.
pub(super) fn execute_task_get(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    let parsed: TaskIdInput = serde_json::from_value(input).context("invalid TaskGet input")?;
    let task = refresh_stored_task(state.session.cwd.as_path(), &parsed.task_id)?
        .ok_or_else(|| anyhow!("unknown task `{}`", parsed.task_id))?;
    Ok(serde_json::to_string_pretty(&task)?)
}

/// Executes the live `TaskList` workflow tool.
pub(super) fn execute_task_list(state: &mut AppState, cwd: &Path, _input: Value) -> Result<String> {
    let store_cwd = state.session.cwd.as_path();
    let mut store = load_store::<TaskStore>(&tasks_path(store_cwd))?;
    let mut changed = false;
    for task in &mut store.tasks {
        let previous = task.clone();
        if let Some(updated) = refresh_stored_task(store_cwd, &task.task_id)? {
            *task = updated;
            changed |= *task != previous;
        }
    }
    if changed {
        save_store(&tasks_path(store_cwd), &store)?;
    }
    Ok(serde_json::to_string_pretty(&store.tasks)?)
}

/// Executes the live `TaskUpdate` workflow tool.
pub(super) fn execute_task_update(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: TaskUpdateInput =
        serde_json::from_value(input).context("invalid TaskUpdate input")?;
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
        if !task
            .blocked_by
            .iter()
            .any(|existing| existing == &blocked_by)
        {
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
pub(super) fn execute_task_stop(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    let parsed: TaskStopInput = serde_json::from_value(input).context("invalid TaskStop input")?;
    let target = parsed
        .task_id
        .or(parsed.shell_id)
        .ok_or_else(|| anyhow!("TaskStop requires task_id or shell_id"))?;

    let store_cwd = state.session.cwd.as_path();
    let mut tasks = load_store::<TaskStore>(&tasks_path(store_cwd))?;
    if let Some(task) = tasks.tasks.iter_mut().find(|task| task.task_id == target) {
        if let Some(process_id) = task.process_id {
            terminate_process(process_id)?;
            let _ = wait_for_process_exit(process_id, 1_000);
            task.process_id = None;
        }
        if let Some(output) = read_task_output(task) {
            task.output = Some(output);
        }
        task.status = "stopped".to_string();
        if task.output.as_deref().unwrap_or_default().trim().is_empty() {
            task.output = Some("Stopped by TaskStop.".to_string());
        }
        let task_id = task.task_id.clone();
        let task_type = task.task_type.clone().unwrap_or_else(|| "task".to_string());
        let command = task.command.clone();
        save_store(&tasks_path(store_cwd), &tasks)?;
        return Ok(serde_json::to_string_pretty(&json!({
            "message": format!("Successfully stopped task: {task_id}"),
            "task_id": task_id,
            "task_type": task_type,
            "command": command,
        }))?);
    }

    let mut agents = load_store::<AgentStore>(&agents_path(store_cwd))?;
    if let Some(agent) = agents
        .agents
        .iter_mut()
        .find(|agent| agent.agent_id == target)
    {
        agent.status = "stopped".to_string();
        append_agent_message(
            Path::new(&agent.output_file),
            &json!("Stopped by TaskStop."),
        )?;
        let output = json!({
            "task_id": target,
            "status": agent.status,
            "output_file": agent.output_file
        });
        save_store(&agents_path(store_cwd), &agents)?;
        return Ok(serde_json::to_string_pretty(&output)?);
    }

    if let Some(process_id) = super::store::parse_shell_task_pid(&target) {
        terminate_process(process_id)?;
        let _ = wait_for_process_exit(process_id, 1_000);
        let output_file = super::store::shell_output_path(store_cwd, &target)?;
        return Ok(serde_json::to_string_pretty(&json!({
            "message": format!("Successfully stopped task: {target}"),
            "task_id": target,
            "task_type": "shell",
            "command": Value::Null,
            "outputFile": output_file.display().to_string()
        }))?);
    }

    bail!("unknown task `{}`", target)
}

/// Executes the live `TaskOutput` workflow tool.
pub(super) fn execute_task_output(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let parsed: TaskOutputInput =
        serde_json::from_value(input).context("invalid TaskOutput input")?;
    let store_cwd = state.session.cwd.as_path();
    let block = parsed.block.unwrap_or(true);
    let timeout = parsed.timeout.unwrap_or(30_000);
    let (task, timed_out) = if block {
        wait_for_stored_task(store_cwd, &parsed.task_id, timeout)?
    } else {
        (refresh_stored_task(store_cwd, &parsed.task_id)?, false)
    };
    if let Some(task) = task {
        return Ok(serde_json::to_string_pretty(&json!({
            "retrieval_status": if timed_out { "timeout" } else if terminal_task_status(&task.status) { "success" } else { "not_ready" },
            "task_id": task.task_id,
            "task_type": task.task_type,
            "status": task.status,
            "output": read_task_output(&task),
            "outputFile": task.output_file,
            "block": block,
            "timeout": timeout
        }))?);
    }
    let agents = load_store::<AgentStore>(&agents_path(store_cwd))?;
    if let Some(agent) = agents
        .agents
        .iter()
        .find(|agent| agent.agent_id == parsed.task_id)
    {
        let output = fs::read_to_string(&agent.output_file).unwrap_or_default();
        return Ok(serde_json::to_string_pretty(&json!({
            "retrieval_status": "success",
            "task_id": agent.agent_id,
            "status": agent.status,
            "output": output,
            "outputFile": agent.output_file,
            "block": block,
            "timeout": timeout
        }))?);
    }

    if let Some(process_id) = super::store::parse_shell_task_pid(&parsed.task_id) {
        let exited = if block {
            wait_for_process_exit(process_id, timeout)
        } else {
            !process_is_running(process_id)
        };
        let output_file = super::store::shell_output_path(store_cwd, &parsed.task_id)?;
        let output = fs::read_to_string(&output_file).unwrap_or_default();
        return Ok(serde_json::to_string_pretty(&json!({
            "retrieval_status": if exited { "success" } else { "timeout" },
            "task_id": parsed.task_id,
            "task_type": "shell",
            "status": if process_is_running(process_id) { "running" } else { "completed" },
            "output": output,
            "outputFile": output_file.display().to_string(),
            "block": block,
            "timeout": timeout
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
    if state.plan_mode {
        let plan_path = ensure_plan_file(state)?;
        return Ok(serde_json::to_string_pretty(&json!({
            "status": "already_in_plan_mode",
            "planMode": true,
            "permissionMode": "plan",
            "planFile": plan_path.display().to_string(),
            "plan": fs::read_to_string(plan_path).unwrap_or_default()
        }))?);
    }
    state.plan_mode = true;
    let plan_path = ensure_plan_file(state)?;
    Ok(serde_json::to_string_pretty(&json!({
        "status": "entered",
        "planMode": true,
        "permissionMode": "plan",
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
    if !state.plan_mode {
        bail!("ExitPlanMode can only be used while plan mode is active");
    }
    state.plan_mode = false;
    let plan_path = ensure_plan_file(state)?;
    let plan = fs::read_to_string(&plan_path).unwrap_or_default();
    Ok(serde_json::to_string_pretty(&json!({
        "status": "exited",
        "planMode": false,
        "planFile": plan_path.display().to_string(),
        "plan": plan,
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
    validate_ask_user_questions(&parsed.questions)?;
    let pending_path = workflow_root(state.session.cwd.as_path())?.join("pending_questions.json");
    fs::write(
        &pending_path,
        serde_json::to_string_pretty(&parsed.questions)?,
    )?;
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
    let mut store = load_store::<WorktreeStore>(&worktrees_path(state.session.cwd.as_path()))?;
    if store
        .worktrees
        .iter()
        .any(|worktree| Path::new(&worktree.path) == cwd)
    {
        bail!("already in a managed worktree session");
    }
    let worktree_name = parsed
        .name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("worktree-{}", Uuid::new_v4().simple()));
    if store
        .worktrees
        .iter()
        .any(|worktree| worktree.name == worktree_name)
    {
        bail!("worktree `{worktree_name}` already exists in this session");
    }
    let base_cwd = cwd.to_path_buf();
    let repo_root = git_toplevel(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let worktree_root = repo_root.join(".worktree");
    fs::create_dir_all(&worktree_root)?;
    let worktree_path = worktree_root.join(&worktree_name);
    if worktree_path.exists() {
        bail!("worktree path {} already exists", worktree_path.display());
    }
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
    store.worktrees.push(StoredWorktree {
        name: worktree_name.clone(),
        path: worktree_path.display().to_string(),
        base_cwd: base_cwd.display().to_string(),
        branch: branch.clone(),
        original_head_commit: git_head_commit(&repo_root)?,
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
    if !matches!(parsed.action.as_str(), "keep" | "remove") {
        bail!("ExitWorktree action must be `keep` or `remove`");
    }
    let mut store = load_store::<WorktreeStore>(&worktrees_path(state.session.cwd.as_path()))?;
    let index = store
        .worktrees
        .iter()
        .position(|worktree| Path::new(&worktree.path) == cwd)
        .ok_or_else(|| anyhow!("no active worktree session found"))?;
    let entry = store.worktrees[index].clone();
    let worktree_path = PathBuf::from(&entry.path);
    let base_cwd = PathBuf::from(&entry.base_cwd);

    if parsed.action == "remove" {
        if is_git_repo(&base_cwd) {
            let dirty = git_dirty(&worktree_path).unwrap_or(false);
            let ahead = entry
                .original_head_commit
                .as_deref()
                .map(|head| git_ahead_count(&worktree_path, head))
                .transpose()?
                .unwrap_or(0);
            if (dirty || ahead > 0) && !parsed.discard_changes {
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
    }
    store.worktrees.remove(index);
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
pub(super) fn execute_config(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    let parsed: ConfigInput = serde_json::from_value(input).context("invalid Config input")?;
    let paths = ConfigPaths::discover(cwd);
    ensure_workspace_dirs(&paths)?;
    let previous = get_config_value(state, &parsed.setting)?;
    let operation = if parsed.value.is_some() { "set" } else { "get" };
    if let Some(value) = parsed.value {
        set_config_value(state, &parsed.setting, value)?;
        save_workspace_config(&paths, &state.config)?;
    }
    let current = get_config_value(state, &parsed.setting)?;
    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "operation": operation,
        "setting": parsed.setting,
        "value": current,
        "previousValue": previous,
        "path": paths.workspace_config_file().display().to_string()
    }))?)
}

/// Executes the live `LSP` workflow tool.
pub(super) fn execute_lsp(_state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
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
        "goToDefinition"
        | "findReferences"
        | "goToImplementation"
        | "prepareCallHierarchy"
        | "incomingCalls"
        | "outgoingCalls" => json!({
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
pub(super) fn execute_powershell(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    let parsed: PowerShellInput =
        serde_json::from_value(input).context("invalid PowerShell input")?;
    let shell = detect_powershell_binary()?;
    if parsed.run_in_background {
        let mut tasks = load_store::<TaskStore>(&tasks_path(state.session.cwd.as_path()))?;
        let task_id = next_task_id(&tasks.tasks);
        let output_file = task_output_path(state.session.cwd.as_path(), &task_id)?;
        let stdout = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&output_file)
            .with_context(|| format!("failed to create {}", output_file.display()))?;
        let stderr = stdout
            .try_clone()
            .with_context(|| format!("failed to clone {}", output_file.display()))?;
        let child = Command::new(&shell)
            .args(["-NoLogo", "-Command", &parsed.command])
            .current_dir(cwd)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .with_context(|| format!("failed to start {}", shell))?;
        tasks.tasks.push(StoredTask {
            task_id: task_id.clone(),
            subject: parsed
                .description
                .clone()
                .unwrap_or_else(|| "PowerShell".to_string()),
            description: parsed.command.clone(),
            active_form: "Running PowerShell command".to_string(),
            status: "running".to_string(),
            owner: None,
            blocks: Vec::new(),
            blocked_by: Vec::new(),
            metadata: Default::default(),
            output: None,
            task_type: Some("powershell".to_string()),
            command: Some(parsed.command.clone()),
            process_id: Some(child.id()),
            output_file: Some(output_file.display().to_string()),
            started_at_ms: Some(now_ms()),
            updated_at_ms: Some(now_ms()),
            exit_code: None,
        });
        save_store(&tasks_path(state.session.cwd.as_path()), &tasks)?;
        return Ok(serde_json::to_string_pretty(&json!({
            "stdout": "",
            "stderr": "",
            "interrupted": false,
            "backgroundTaskId": task_id,
            "outputFile": output_file.display().to_string(),
            "processId": child.id(),
            "dangerouslyDisableSandbox": parsed.dangerously_disable_sandbox
        }))?);
    }

    let timeout_ms = parsed.timeout.unwrap_or(120_000).clamp(1, 600_000);
    let child = Command::new(&shell)
        .args(["-NoLogo", "-Command", &parsed.command])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to execute {}", shell))?;
    let timed = wait_for_child_output(child, timeout_ms)
        .with_context(|| format!("failed to execute {}", shell))?;
    state.record_task(
        parsed
            .description
            .clone()
            .unwrap_or_else(|| "PowerShell".to_string()),
        parsed.command.clone(),
        !timed.timed_out,
    );
    Ok(serde_json::to_string_pretty(&json!({
        "stdout": timed.stdout,
        "stderr": timed.stderr,
        "interrupted": timed.timed_out,
        "dangerouslyDisableSandbox": parsed.dangerously_disable_sandbox,
        "timeoutMs": timeout_ms
    }))?)
}
