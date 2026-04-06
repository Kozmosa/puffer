use crate::{AppState, MessageRole, RenderedMessage};
use anyhow::Result;
use arboard::Clipboard;
use puffer_provider_registry::ProviderRegistry;
use puffer_resources::{
    plugin_by_id, plugin_mcp_servers, skill_by_name, LoadedResources,
};
use puffer_session_store::{SessionStore, TranscriptEvent};
use puffer_tools::ToolRegistry;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

pub(crate) fn list_skills(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    if resources.skills.is_empty() {
        return emit_system(state, session_store, "No skills are available.".to_string());
    }
    let mut text = String::from("Available skills:\n");
    for skill in &resources.skills {
        let _ = writeln!(
            &mut text,
            "/skill:{} - {}",
            skill.value.name, skill.value.description
        );
    }
    emit_system(state, session_store, text)
}

pub(crate) fn describe_permissions(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    let registry = ToolRegistry::from_resources(resources);
    if registry.tools().count() == 0 {
        return emit_system(
            state,
            session_store,
            "No tool metadata is loaded.".to_string(),
        );
    }

    let mut text = String::from("Tool permission summary:\n");
    for tool in registry.tools() {
        let _ = writeln!(
            &mut text,
            "- {} [{}]: approval={} sandbox={}",
            tool.spec.name,
            tool.spec.handler,
            tool.spec.approval_policy.as_deref().unwrap_or("<unspecified>"),
            tool.spec.sandbox_policy.as_deref().unwrap_or("<unspecified>")
        );
    }
    emit_system(state, session_store, text)
}

pub(crate) fn run_doctor(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    session_store: &SessionStore,
) -> Result<()> {
    let registry = ToolRegistry::from_resources(resources);
    let mut text = String::from("Puffer doctor summary:\n");
    let _ = writeln!(
        &mut text,
        "provider={} model={}",
        state.current_provider.as_deref().unwrap_or("<unset>"),
        state.current_model.as_deref().unwrap_or("<unset>")
    );
    let _ = writeln!(&mut text, "tool_count={}", registry.tools().count());
    let _ = writeln!(&mut text, "provider_count={}", providers.providers().count());
    let _ = writeln!(&mut text, "working_dirs={}", state.working_dirs.len());
    let _ = writeln!(&mut text, "transcript_messages={}", state.transcript.len());
    emit_system(state, session_store, text)
}

pub(crate) fn describe_plugin(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    if args.is_empty() {
        if resources.plugins.is_empty() {
            return emit_system(state, session_store, "No plugins are installed.".to_string());
        }
        let mut text = String::from("Plugins:\n");
        for plugin in &resources.plugins {
            let _ = writeln!(
                &mut text,
                "{} - {}",
                plugin.value.id, plugin.value.description
            );
        }
        return emit_system(state, session_store, text);
    }
    let Some(plugin) = plugin_by_id(resources, args) else {
        return emit_system(state, session_store, format!("Unknown plugin {args}."));
    };
    let mut text = format!(
        "Plugin {}\n{}\n",
        plugin.value.id, plugin.value.description
    );
    if !plugin.value.commands.is_empty() {
        let commands = plugin
            .value
            .commands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(&mut text, "Commands: {commands}");
    }
    if !plugin.value.skills.is_empty() {
        let _ = writeln!(&mut text, "Skills: {}", plugin.value.skills.join(", "));
    }
    if !plugin.value.mcp_servers.is_empty() {
        let ids = plugin
            .value
            .mcp_servers
            .iter()
            .map(|server| server.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(&mut text, "MCP servers: {ids}");
    }
    emit_system(state, session_store, text)
}

pub(crate) fn list_mcp_servers(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    let servers = plugin_mcp_servers(resources);
    if servers.is_empty() && resources.mcp_servers.is_empty() {
        return emit_system(
            state,
            session_store,
            "No MCP servers are configured.".to_string(),
        );
    }
    let mut text = String::from("MCP servers:\n");
    for server in &resources.mcp_servers {
        let _ = writeln!(
            &mut text,
            "{} [{}] -> {}",
            server.value.id,
            server.value.transport,
            server.value.endpoint
        );
    }
    for (plugin, server) in servers {
        let target = if server.target.is_empty() {
            server.endpoint.as_str()
        } else {
            server.target.as_str()
        };
        let _ = writeln!(
            &mut text,
            "{}:{} [{}] -> {}",
            plugin.id, server.id, server.transport, target
        );
    }
    emit_system(state, session_store, text)
}

pub(crate) fn list_ides(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    if resources.ides.is_empty() {
        return emit_system(
            state,
            session_store,
            "No IDE integrations are configured.".to_string(),
        );
    }
    let mut text = String::from("IDE integrations:\n");
    for ide in &resources.ides {
        let _ = writeln!(
            &mut text,
            "{} - {}",
            ide.value.display_name,
            ide.value.description
        );
    }
    emit_system(state, session_store, text)
}

pub(crate) fn copy_last_message(
    state: &mut AppState,
    session_store: &SessionStore,
) -> Result<()> {
    let last = state
        .transcript
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::Assistant)
        .map(|message| message.text.clone())
        .unwrap_or_default();
    if last.is_empty() {
        return emit_system(
            state,
            session_store,
            "No assistant response is available to copy.".to_string(),
        );
    }

    match Clipboard::new().and_then(|mut clipboard| clipboard.set_text(last.clone())) {
        Ok(()) => emit_system(state, session_store, "Copied the latest assistant response.".to_string()),
        Err(_) => emit_system(
            state,
            session_store,
            format!("Latest assistant response:\n{last}"),
        ),
    }
}

pub(crate) fn describe_context(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    emit_system(
        state,
        session_store,
        format!(
            "Context summary:\ntranscript_messages={}\nworking_dirs={}\nprompts={}\nskills={}\nplugins={}",
            state.transcript.len(),
            state.working_dirs.len(),
            resources.prompts.len(),
            resources.skills.len(),
            resources.plugins.len()
        ),
    )
}

pub(crate) fn describe_git_diff(
    state: &mut AppState,
    session_store: &SessionStore,
) -> Result<()> {
    emit_system(state, session_store, render_git_diff_summary(&state.cwd))
}

pub(crate) fn execute_skill_command(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    skill_name: &str,
) -> Result<()> {
    if let Some(skill) = skill_by_name(resources, skill_name) {
        emit_system(
            state,
            session_store,
            format!(
                "Skill {}\n{}\n\n{}",
                skill.value.name, skill.value.description, skill.value.content
            ),
        )
    } else {
        emit_system(state, session_store, format!("Unknown skill {skill_name}."))
    }
}

pub(crate) fn emit_system(
    state: &mut AppState,
    session_store: &SessionStore,
    text: String,
) -> Result<()> {
    state.push_message(MessageRole::System, text.clone());
    session_store.append_event(state.session.id, TranscriptEvent::SystemMessage { text })?;
    Ok(())
}

fn render_git_diff_summary(cwd: &PathBuf) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["status", "--short"])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.trim().is_empty() {
                "Working tree is clean.".to_string()
            } else {
                format!("Git status:\n{}", stdout.trim_end())
            }
        }
        Ok(output) => format!(
            "Failed to read git status: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => format!("Failed to run git status: {error}"),
    }
}

pub(crate) fn rewind_transcript(
    state: &mut AppState,
    session_store: &SessionStore,
) -> Result<()> {
    if state.transcript.is_empty() {
        return emit_system(state, session_store, "Transcript is already empty.".to_string());
    }
    state.transcript.pop();
    emit_system(
        state,
        session_store,
        "Removed the latest rendered transcript item.".to_string(),
    )
}

pub(crate) fn terminal_setup_advice(state: &AppState) -> String {
    format!(
        "Terminal setup:\n- current cwd: {}\n- no_alt_screen: {}\n- tmux_golden_mode: {}",
        state.cwd.display(),
        state.config.ui.no_alt_screen,
        state.config.ui.tmux_golden_mode
    )
}
