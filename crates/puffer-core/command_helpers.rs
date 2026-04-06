use crate::{AppState, MessageRole, ToolInvocation};
use anyhow::Result;
use arboard::Clipboard;
use puffer_config::{ensure_workspace_dirs, ConfigPaths};
use puffer_provider_registry::ProviderRegistry;
use puffer_resources::{plugin_by_id, plugin_mcp_servers, skill_by_name, LoadedResources};
use puffer_session_store::{SessionStore, TranscriptEvent};
use puffer_tools::ToolRegistry;
use serde::Deserialize;
use std::fs;
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
            tool.spec
                .policy
                .approval_policy
                .as_deref()
                .unwrap_or("<unspecified>"),
            tool.spec
                .policy
                .sandbox_policy
                .as_deref()
                .unwrap_or("<unspecified>")
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
    let _ = writeln!(
        &mut text,
        "provider_count={}",
        providers.providers().count()
    );
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
            return emit_system(
                state,
                session_store,
                "No plugins are installed.".to_string(),
            );
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
    let mut text = format!("Plugin {}\n{}\n", plugin.value.id, plugin.value.description);
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
            server.value.id, server.value.transport, server.value.endpoint
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
            ide.value.display_name, ide.value.description
        );
    }
    emit_system(state, session_store, text)
}

pub(crate) fn copy_last_message(state: &mut AppState, session_store: &SessionStore) -> Result<()> {
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
        Ok(()) => emit_system(
            state,
            session_store,
            "Copied the latest assistant response.".to_string(),
        ),
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

pub(crate) fn describe_git_diff(state: &mut AppState, session_store: &SessionStore) -> Result<()> {
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

pub(crate) fn rewind_transcript(state: &mut AppState, session_store: &SessionStore) -> Result<()> {
    if state.transcript.is_empty() {
        return emit_system(
            state,
            session_store,
            "Transcript is already empty.".to_string(),
        );
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

pub(crate) fn handle_config_command(
    state: &mut AppState,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let config_path = paths.workspace_config_file();
    let trimmed = args.trim();

    if trimmed.is_empty() || trimmed == "show" {
        return emit_system(
            state,
            session_store,
            format!(
                "Config summary:\npath={}\napp_name={}\ndefault_provider={}\ndefault_model={}\ntheme={}\nno_alt_screen={}\ntmux_golden_mode={}",
                config_path.display(),
                state.config.app_name,
                state.config.default_provider.as_deref().unwrap_or("<unset>"),
                state.config.default_model.as_deref().unwrap_or("<unset>"),
                state.config.theme,
                state.config.ui.no_alt_screen,
                state.config.ui.tmux_golden_mode,
            ),
        );
    }

    if trimmed == "path" {
        return emit_system(
            state,
            session_store,
            format!("Workspace config path: {}", config_path.display()),
        );
    }

    let Some(rest) = trimmed.strip_prefix("set ") else {
        return emit_system(
            state,
            session_store,
            "Usage: /config [show|path|set <theme|default_provider|default_model|no_alt_screen|tmux_golden_mode> <value>]".to_string(),
        );
    };
    let Some((key, value)) = rest.split_once(' ') else {
        return emit_system(
            state,
            session_store,
            "Usage: /config set <key> <value>".to_string(),
        );
    };
    let value = value.trim();
    match key {
        "theme" => state.config.theme = value.to_string(),
        "default_provider" => state.config.default_provider = Some(value.to_string()),
        "default_model" => state.config.default_model = Some(value.to_string()),
        "no_alt_screen" => state.config.ui.no_alt_screen = parse_bool(value)?,
        "tmux_golden_mode" => state.config.ui.tmux_golden_mode = parse_bool(value)?,
        _ => {
            return emit_system(
                state,
                session_store,
                format!("Unsupported config key {key}."),
            );
        }
    }
    write_workspace_config(state, &config_path)?;
    emit_system(
        state,
        session_store,
        format!("Updated {key} in {}.", config_path.display()),
    )
}

pub(crate) fn handle_keybindings_command(
    state: &mut AppState,
    session_store: &SessionStore,
) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let keybindings_path = paths.workspace_config_dir.join("keybindings.toml");
    if !keybindings_path.exists() {
        fs::write(&keybindings_path, default_keybindings_contents())?;
    }
    emit_system(
        state,
        session_store,
        format!(
            "Keybindings file: {}\n{}",
            keybindings_path.display(),
            fs::read_to_string(&keybindings_path)?
        ),
    )
}

pub(crate) fn handle_hooks_command(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let hooks_dir = paths.workspace_config_dir.join("resources/hooks");
    fs::create_dir_all(&hooks_dir)?;
    let hooks_path = hooks_dir.join("tool_end.yaml");
    if !hooks_path.exists() {
        fs::write(&hooks_path, default_hooks_contents())?;
    }
    if args.trim() == "path" {
        return emit_system(
            state,
            session_store,
            format!("Hooks directory: {}", hooks_dir.display()),
        );
    }
    emit_system(
        state,
        session_store,
        format!(
            "Hooks directory: {}\nloaded_hooks={}\n{}{}",
            hooks_dir.display(),
            resources.hooks.len(),
            if resources.hooks.is_empty() {
                format!("Example hook file: {}\n", hooks_path.display())
            } else {
                let mut summary = String::from("Loaded hooks:\n");
                for hook in &resources.hooks {
                    let _ = writeln!(
                        &mut summary,
                        "- {} [{}] -> {}",
                        hook.value.id, hook.value.event, hook.value.command
                    );
                }
                summary
            },
            fs::read_to_string(&hooks_path)?
        ),
    )
}

pub(crate) fn handle_agents_command(
    state: &mut AppState,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let agents_path = paths.workspace_config_dir.join("agents.yaml");
    if !agents_path.exists() {
        fs::write(
            &agents_path,
            default_agents_contents(state.current_model.as_deref()),
        )?;
    }
    let trimmed = args.trim();
    if trimmed == "path" {
        return emit_system(
            state,
            session_store,
            format!("Agents file: {}", agents_path.display()),
        );
    }
    let contents = fs::read_to_string(&agents_path)?;
    let parsed = parse_agents_file(&contents)?;
    match trimmed {
        "" | "show" => emit_system(
            state,
            session_store,
            format!("Agents file: {}\n{}", agents_path.display(), contents),
        ),
        "list" => {
            let mut text = String::from("Agents:\n");
            for agent in parsed.agents {
                let _ = writeln!(
                    &mut text,
                    "- {} role={} model={}",
                    agent.id, agent.role, agent.model
                );
            }
            emit_system(state, session_store, text)
        }
        _ if trimmed.starts_with("show ") => {
            let agent_id = trimmed.trim_start_matches("show ").trim();
            if let Some(agent) = parsed.agents.iter().find(|agent| agent.id == agent_id) {
                emit_system(
                    state,
                    session_store,
                    format!(
                        "Agent {}\nrole={}\nmodel={}",
                        agent.id, agent.role, agent.model
                    ),
                )
            } else {
                emit_system(state, session_store, format!("Unknown agent {agent_id}."))
            }
        }
        _ if trimmed.starts_with("use ") => {
            let agent_id = trimmed.trim_start_matches("use ").trim();
            if let Some(agent) = parsed.agents.iter().find(|agent| agent.id == agent_id) {
                state.current_model = Some(agent.model.clone());
                state.current_provider = agent
                    .model
                    .split_once('/')
                    .map(|(provider, _)| provider.to_string())
                    .or_else(|| state.current_provider.clone());
                emit_system(
                    state,
                    session_store,
                    format!(
                        "Selected agent {}.\nrole={}\nmodel={}",
                        agent.id, agent.role, agent.model
                    ),
                )
            } else {
                emit_system(state, session_store, format!("Unknown agent {agent_id}."))
            }
        }
        _ => emit_system(
            state,
            session_store,
            "Usage: /agents [path|list|show <id>|use <id>]".to_string(),
        ),
    }
}

pub(crate) fn append_tool_invocations(
    state: &mut AppState,
    session_store: &SessionStore,
    invocations: &[ToolInvocation],
) -> Result<()> {
    for invocation in invocations {
        state.record_task(
            invocation.tool_id.clone(),
            invocation.input.clone(),
            invocation.success,
        );
        emit_system(state, session_store, format_tool_invocation(invocation))?;
    }
    Ok(())
}

pub(crate) fn handle_memory_command(
    state: &mut AppState,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    let trimmed = args.trim();
    if trimmed.is_empty() || trimmed == "show" {
        return emit_system(state, session_store, render_memory_summary(state));
    }

    if trimmed == "clear" {
        let tags = state.session.tags.clone();
        session_store.set_note(state.session.id, None)?;
        session_store.set_slug(state.session.id, None)?;
        for tag in &tags {
            session_store.remove_tag(state.session.id, tag)?;
        }
        state.session.note = None;
        state.session.slug = None;
        state.session.tags.clear();
        return emit_system(
            state,
            session_store,
            "Cleared session note, slug, and tags.".to_string(),
        );
    }

    if let Some(rest) = trimmed.strip_prefix("note ") {
        if matches!(rest, "clear" | "none" | "off") {
            session_store.set_note(state.session.id, None)?;
            state.session.note = None;
            return emit_system(state, session_store, "Cleared session note.".to_string());
        }
        session_store.set_note(state.session.id, Some(rest.to_string()))?;
        state.session.note = Some(rest.to_string());
        return emit_system(state, session_store, format!("Session note set to `{rest}`."));
    }

    if let Some(rest) = trimmed.strip_prefix("slug ") {
        if matches!(rest, "clear" | "none" | "off") {
            session_store.set_slug(state.session.id, None)?;
            state.session.slug = None;
            return emit_system(state, session_store, "Cleared session slug.".to_string());
        }
        session_store.set_slug(state.session.id, Some(rest.to_string()))?;
        state.session.slug = Some(rest.to_string());
        return emit_system(state, session_store, format!("Session slug set to `{rest}`."));
    }

    if let Some(rest) = trimmed.strip_prefix("tag add ") {
        let tag = rest.trim();
        if tag.is_empty() {
            return emit_system(
                state,
                session_store,
                "Usage: /memory tag add <tag>".to_string(),
            );
        }
        session_store.add_tag(state.session.id, tag)?;
        if !state.session.tags.iter().any(|existing| existing == tag) {
            state.session.tags.push(tag.to_string());
            state.session.tags.sort();
        }
        return emit_system(state, session_store, format!("Added session tag `{tag}`."));
    }

    if let Some(rest) = trimmed.strip_prefix("tag remove ") {
        let tag = rest.trim();
        if tag.is_empty() {
            return emit_system(
                state,
                session_store,
                "Usage: /memory tag remove <tag>".to_string(),
            );
        }
        session_store.remove_tag(state.session.id, tag)?;
        state.session.tags.retain(|existing| existing != tag);
        return emit_system(state, session_store, format!("Removed session tag `{tag}`."));
    }

    emit_system(
        state,
        session_store,
        "Usage: /memory [show|clear|note <text>|note clear|slug <value>|slug clear|tag add <tag>|tag remove <tag>]".to_string(),
    )
}

pub(crate) fn handle_plugin_command(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let plugins_dir = paths.workspace_config_dir.join("resources/plugins");
    fs::create_dir_all(&plugins_dir)?;
    let plugin_path = plugins_dir.join("workspace.yaml");
    if !plugin_path.exists() {
        fs::write(&plugin_path, default_plugin_contents())?;
    }
    if args.trim() == "path" {
        return emit_system(
            state,
            session_store,
            format!("Plugins directory: {}", plugins_dir.display()),
        );
    }
    if !args.trim().is_empty() && args.trim() != "show" {
        return describe_plugin(state, resources, session_store, args);
    }
    emit_system(
        state,
        session_store,
        format!(
            "Plugins directory: {}\nloaded_plugins={}\n{}{}",
            plugins_dir.display(),
            resources.plugins.len(),
            if resources.plugins.is_empty() {
                format!("Example plugin file: {}\n", plugin_path.display())
            } else {
                let mut summary = String::from("Loaded plugins:\n");
                for plugin in &resources.plugins {
                    let _ = writeln!(
                        &mut summary,
                        "- {} -> {}",
                        plugin.value.id, plugin.value.display_name
                    );
                }
                summary
            },
            fs::read_to_string(&plugin_path)?
        ),
    )
}

pub(crate) fn handle_mcp_command(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let mcp_dir = paths.workspace_config_dir.join("resources/mcp_servers");
    fs::create_dir_all(&mcp_dir)?;
    let server_path = mcp_dir.join("workspace.yaml");
    if !server_path.exists() {
        fs::write(&server_path, default_mcp_contents())?;
    }
    if args.trim() == "path" {
        return emit_system(
            state,
            session_store,
            format!("MCP directory: {}", mcp_dir.display()),
        );
    }
    if !args.trim().is_empty() && args.trim() != "show" {
        return list_mcp_servers(state, resources, session_store);
    }
    let mut summary = String::new();
    if resources.mcp_servers.is_empty() && plugin_mcp_servers(resources).is_empty() {
        let _ = writeln!(&mut summary, "Example MCP file: {}", server_path.display());
    } else {
        let _ = writeln!(&mut summary, "Loaded MCP servers:");
        for server in &resources.mcp_servers {
            let _ = writeln!(
                &mut summary,
                "- {} -> {}",
                server.value.id, server.value.display_name
            );
        }
        for (plugin, server) in plugin_mcp_servers(resources) {
            let _ = writeln!(
                &mut summary,
                "- {}:{} -> {}",
                plugin.id, server.id, server.display_name
            );
        }
    }
    emit_system(
        state,
        session_store,
        format!(
            "MCP directory: {}\n{}{}",
            mcp_dir.display(),
            summary,
            fs::read_to_string(&server_path)?
        ),
    )
}

pub(crate) fn handle_ide_command(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let ide_dir = paths.workspace_config_dir.join("resources/ides");
    fs::create_dir_all(&ide_dir)?;
    let ide_path = ide_dir.join("workspace.yaml");
    if !ide_path.exists() {
        fs::write(&ide_path, default_ide_contents())?;
    }
    if args.trim() == "path" {
        return emit_system(
            state,
            session_store,
            format!("IDE directory: {}", ide_dir.display()),
        );
    }
    if args.trim() == "list" {
        return list_ides(state, resources, session_store);
    }
    if args.trim() == "open" {
        return emit_system(
            state,
            session_store,
            format!("Open your IDE integration from {}.", ide_dir.display()),
        );
    }
    emit_system(
        state,
        session_store,
        format!(
            "IDE directory: {}\nloaded_ides={}\n{}{}",
            ide_dir.display(),
            resources.ides.len(),
            if resources.ides.is_empty() {
                format!("Example IDE file: {}\n", ide_path.display())
            } else {
                let mut summary = String::from("Loaded IDE integrations:\n");
                for ide in &resources.ides {
                    let _ = writeln!(
                        &mut summary,
                        "- {} -> {}",
                        ide.value.id, ide.value.display_name
                    );
                }
                summary
            },
            fs::read_to_string(&ide_path)?
        ),
    )
}

pub(crate) fn reload_plugins_summary(
    state: &AppState,
    resources: &LoadedResources,
) -> Result<String> {
    let paths = ConfigPaths::discover(&state.cwd);
    let plugins_dir = paths.workspace_config_dir.join("resources/plugins");
    Ok(format!(
        "Reloaded plugin registry for this session.\nplugins={}\nskills={}\nmcp_servers={}\nsource_dir={}",
        resources.plugins.len(),
        resources.skills.len(),
        resources.mcp_servers.len(),
        plugins_dir.display()
    ))
}

fn format_tool_invocation(invocation: &ToolInvocation) -> String {
    let status = if invocation.success { "ok" } else { "error" };
    let output = invocation.output.trim();
    if output.is_empty() {
        format!("Tool {} [{}]\ninput: {}", invocation.tool_id, status, invocation.input)
    } else {
        format!(
            "Tool {} [{}]\ninput: {}\n{}",
            invocation.tool_id, status, invocation.input, output
        )
    }
}

fn render_memory_summary(state: &AppState) -> String {
    format!(
        "Session memory summary:\nslug={}\nnote={}\ntags={}",
        state.session.slug.as_deref().unwrap_or("<none>"),
        state.session.note.as_deref().unwrap_or("<none>"),
        if state.session.tags.is_empty() {
            "<none>".to_string()
        } else {
            state.session.tags.join(", ")
        },
    )
}

fn parse_bool(value: &str) -> Result<bool> {
    match value {
        "true" | "on" | "1" => Ok(true),
        "false" | "off" | "0" => Ok(false),
        _ => anyhow::bail!("expected a boolean value, got `{value}`"),
    }
}

fn write_workspace_config(state: &AppState, path: &PathBuf) -> Result<()> {
    fs::write(path, toml::to_string_pretty(&state.config)?)?;
    Ok(())
}

fn default_keybindings_contents() -> &'static str {
    "submit = \"enter\"\nclear_input = \"esc\"\nexit = \"ctrl+c\"\n"
}

fn default_hooks_contents() -> &'static str {
    "id: tool-end\n\
event: tool_end\n\
command: echo \"$PUFFER_TOOL_ID:$PUFFER_TOOL_SUCCESS\"\n"
}

fn default_agents_contents(model: Option<&str>) -> String {
    format!(
        "agents:\n  - id: default\n    role: coding\n    model: {}\n",
        model.unwrap_or("anthropic/claude-sonnet-4-5")
    )
}

fn default_plugin_contents() -> &'static str {
    "id: workspace\n\
display_name: Workspace Plugin\n\
description: Customize plugin commands for this workspace.\n\
commands:\n\
  - name: demo\n\
    description: Example command\n"
}

fn default_mcp_contents() -> &'static str {
    "id: workspace\n\
display_name: Workspace MCP\n\
transport: stdio\n\
endpoint: \"\"\n\
target: workspace\n\
description: Example MCP server\n"
}

fn default_ide_contents() -> &'static str {
    "id: workspace\n\
display_name: Workspace IDE\n\
description: Example IDE integration\n"
}

fn parse_agents_file(raw: &str) -> Result<AgentsFile> {
    Ok(serde_yaml::from_str(raw)?)
}

#[derive(Debug, Clone, Deserialize)]
struct AgentsFile {
    agents: Vec<AgentEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct AgentEntry {
    id: String,
    role: String,
    model: String,
}
