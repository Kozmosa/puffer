use crate::{AppState, MessageRole};
use anyhow::{anyhow, bail, Context, Result};
use puffer_config::{ensure_workspace_dirs, ConfigPaths};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::{agent_by_id, LoadedResources};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
struct AgentToolInput {
    description: String,
    prompt: String,
    #[serde(default)]
    subagent_type: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    run_in_background: bool,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    isolation: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct AgentCompletedOutput {
    status: &'static str,
    #[serde(rename = "agentId")]
    agent_id: String,
    #[serde(rename = "agentType")]
    agent_type: String,
    description: String,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(rename = "toolUses")]
    tool_uses: usize,
    result: String,
}

#[derive(Debug, Serialize)]
struct AgentAsyncOutput {
    status: &'static str,
    #[serde(rename = "agentId")]
    agent_id: String,
    #[serde(rename = "agentType")]
    agent_type: String,
    description: String,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(rename = "outputFile")]
    output_file: String,
    #[serde(rename = "canReadOutputFile")]
    can_read_output_file: bool,
}

#[derive(Debug)]
struct PreparedAgentExecution {
    agent_id: String,
    agent_type: String,
    description: String,
    prompt: String,
    name: Option<String>,
    run_in_background: bool,
    nested_cwd: PathBuf,
    nested_state: AppState,
    nested_resources: LoadedResources,
    resolved_model: Option<String>,
}

/// Executes the runtime-backed `Agent` tool by running a nested model turn.
pub(super) fn execute_agent_tool(
    state: &AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &mut AuthStore,
    cwd: &Path,
    input: Value,
) -> Result<String> {
    let input: AgentToolInput = serde_json::from_value(input).context("invalid Agent input")?;
    if input.prompt.trim().is_empty() {
        bail!("Agent prompt cannot be empty");
    }
    if input.isolation.is_some() {
        bail!("agent isolation is not implemented in this runtime");
    }

    let prepared = prepare_agent_execution(state, resources, providers, cwd, input)?;
    if prepared.prompt.trim().is_empty() {
        bail!("Agent prompt cannot be empty");
    }
    if prepared.nested_state.current_provider.is_none() && providers.providers().next().is_none() {
        bail!("no providers are registered");
    }

    if prepared.run_in_background {
        return launch_background_agent(prepared, providers.clone(), auth_store.clone());
    }

    run_agent_synchronously(prepared, providers, auth_store)
}

fn prepare_agent_execution(
    state: &AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    cwd: &Path,
    input: AgentToolInput,
) -> Result<PreparedAgentExecution> {
    let selected_agent = input
        .subagent_type
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("general-purpose");
    let agent = agent_by_id(resources, selected_agent)
        .or_else(|| {
            resources
                .agents
                .iter()
                .find(|item| item.value.id.eq_ignore_ascii_case(selected_agent))
        })
        .ok_or_else(|| {
            let available = resources
                .agents
                .iter()
                .map(|item| item.value.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow!("unknown agent `{selected_agent}`. Available agents: {available}")
        })?;

    let nested_cwd = resolve_agent_cwd(cwd, input.cwd.as_deref())?;
    let nested_resources = filter_resources_for_agent(resources, &agent.value.tools);
    let mut nested_state = state.clone();
    nested_state.cwd = nested_cwd.clone();
    nested_state.transcript.clear();
    nested_state.push_message(MessageRole::System, agent.value.prompt.trim().to_string());

    if let Some(model) = input
        .model
        .as_deref()
        .or(agent.value.model.as_deref())
        .filter(|value| !value.trim().is_empty())
    {
        let resolved = providers.resolve_model(model);
        nested_state.current_model = Some(model.to_string());
        nested_state.current_provider = resolved
            .map(|descriptor| descriptor.provider.clone())
            .or_else(|| {
                model
                    .split_once('/')
                    .map(|(provider, _)| provider.to_string())
            })
            .or_else(|| state.current_provider.clone());
    }
    Ok(PreparedAgentExecution {
        agent_id: format!("agent-{}", Uuid::new_v4().simple()),
        agent_type: agent.value.id.clone(),
        description: input.description.trim().to_string(),
        prompt: input.prompt,
        name: input
            .name
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        run_in_background: input.run_in_background,
        nested_cwd,
        resolved_model: nested_state.current_model.clone(),
        nested_state,
        nested_resources,
    })
}

fn run_agent_synchronously(
    mut prepared: PreparedAgentExecution,
    providers: &ProviderRegistry,
    auth_store: &mut AuthStore,
) -> Result<String> {
    let turn = super::execute_user_prompt(
        &mut prepared.nested_state,
        &prepared.nested_resources,
        providers,
        auth_store,
        &prepared.prompt,
    )?;
    let payload = AgentCompletedOutput {
        status: "completed",
        agent_id: prepared.agent_id,
        agent_type: prepared.agent_type,
        description: prepared.description,
        prompt: prepared.prompt,
        name: prepared.name,
        cwd: prepared.nested_cwd.display().to_string(),
        model: prepared.resolved_model,
        tool_uses: turn.tool_invocations.len(),
        result: turn.assistant_text.trim().to_string(),
    };
    Ok(serde_json::to_string_pretty(&payload)?)
}

fn launch_background_agent(
    prepared: PreparedAgentExecution,
    providers: ProviderRegistry,
    auth_store: AuthStore,
) -> Result<String> {
    let output_file = agent_output_path(&prepared.nested_state.session.cwd, &prepared.agent_id)?;
    fs::write(
        &output_file,
        serde_json::to_string_pretty(&json!({
            "status": "running",
            "agentId": prepared.agent_id,
            "agentType": prepared.agent_type,
            "description": prepared.description,
            "prompt": prepared.prompt,
            "name": prepared.name,
            "cwd": prepared.nested_cwd.display().to_string(),
            "model": prepared.resolved_model,
        }))?,
    )
    .with_context(|| format!("failed to initialize {}", output_file.display()))?;

    let response = AgentAsyncOutput {
        status: "async_launched",
        agent_id: prepared.agent_id.clone(),
        agent_type: prepared.agent_type.clone(),
        description: prepared.description.clone(),
        prompt: prepared.prompt.clone(),
        name: prepared.name.clone(),
        cwd: prepared.nested_cwd.display().to_string(),
        model: prepared.resolved_model.clone(),
        output_file: output_file.display().to_string(),
        can_read_output_file: true,
    };

    thread::spawn(move || {
        let mut nested_state = prepared.nested_state;
        let nested_resources = prepared.nested_resources;
        let prompt = prepared.prompt.clone();
        let result = {
            let mut nested_auth_store = auth_store;
            super::execute_user_prompt(
                &mut nested_state,
                &nested_resources,
                &providers,
                &mut nested_auth_store,
                &prompt,
            )
        };
        let final_payload = match result {
            Ok(turn) => json!(AgentCompletedOutput {
                status: "completed",
                agent_id: prepared.agent_id,
                agent_type: prepared.agent_type,
                description: prepared.description,
                prompt: prepared.prompt,
                name: prepared.name,
                cwd: prepared.nested_cwd.display().to_string(),
                model: prepared.resolved_model,
                tool_uses: turn.tool_invocations.len(),
                result: turn.assistant_text.trim().to_string(),
            }),
            Err(error) => json!({
                "status": "failed",
                "agentId": prepared.agent_id,
                "agentType": prepared.agent_type,
                "description": prepared.description,
                "prompt": prepared.prompt,
                "name": prepared.name,
                "cwd": prepared.nested_cwd.display().to_string(),
                "model": prepared.resolved_model,
                "error": error.to_string(),
            }),
        };
        let _ = fs::write(
            &output_file,
            serde_json::to_string_pretty(&final_payload)
                .unwrap_or_else(|_| "{\"status\":\"failed\"}".to_string()),
        );
    });

    Ok(serde_json::to_string_pretty(&response)?)
}

fn resolve_agent_cwd(parent_cwd: &Path, override_cwd: Option<&str>) -> Result<PathBuf> {
    let Some(override_cwd) = override_cwd.filter(|value| !value.trim().is_empty()) else {
        return Ok(parent_cwd.to_path_buf());
    };
    let requested = PathBuf::from(override_cwd.trim());
    let resolved = if requested.is_absolute() {
        requested
    } else {
        parent_cwd.join(requested)
    };
    let metadata = std::fs::metadata(&resolved)
        .with_context(|| format!("agent cwd {} does not exist", resolved.display()))?;
    if !metadata.is_dir() {
        bail!("agent cwd {} is not a directory", resolved.display());
    }
    Ok(resolved)
}

fn agent_output_path(session_cwd: &Path, agent_id: &str) -> Result<PathBuf> {
    let paths = ConfigPaths::discover(session_cwd);
    ensure_workspace_dirs(&paths)?;
    let dir = paths
        .workspace_config_dir
        .join("runtime")
        .join("agent_outputs");
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    Ok(dir.join(format!("{agent_id}.json")))
}

fn filter_resources_for_agent(resources: &LoadedResources, tools: &[String]) -> LoadedResources {
    let mut filtered = resources.clone();
    let wildcard = tools.is_empty() || tools.iter().any(|tool| tool == "*");
    filtered.tools.retain(|tool| {
        if tool.value.id.eq_ignore_ascii_case("Agent") {
            return false;
        }
        wildcard
            || tools
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(&tool.value.id))
    });
    filtered
}
