use super::common::open_text_file_in_editor;
use super::emit_system;
use super::CommandActionEntry;
use crate::config_settings::{
    config_setting_scope, parse_config_cli_value, persist_config_setting,
    persist_user_settings as persist_user_config_settings, render_supported_config_key_list,
    scope_label, set_config_value as set_state_config_value,
};
use crate::permissions::{
    load_or_initialize_permissions, load_or_initialize_sandbox_settings, normalize_tool_id,
    write_permissions, write_sandbox_settings, PermissionsSettings, SandboxSettings,
};
use crate::AppState;
use anyhow::Result;
use puffer_config::{ensure_workspace_dirs, load_config, ConfigPaths};
use puffer_resources::{hook_by_id, LoadedResources};
use puffer_session_store::SessionStore;
use puffer_tools::ToolRegistry;
use serde_json::json;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

/// Summarizes loaded tool approval and sandbox metadata.
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

/// Renders the current workspace config summary without mutating transcript state.
pub(crate) fn render_config_summary(state: &AppState) -> Result<String> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let mut text = String::new();
    let _ = writeln!(
        &mut text,
        "Config summary\nuser_config_path={}\nworkspace_config_path={}\napp_name={}",
        paths.user_config_file().display(),
        paths.workspace_config_file().display(),
        state.config.app_name
    );
    let _ = writeln!(
        &mut text,
        "\n[user]\ndefault_provider={}\ndefault_model={}\ntheme={}\neditor_mode={}\nfast_mode={}\ncopy_full_response={}\neffort_level={}",
        state.config.default_provider.as_deref().unwrap_or("<unset>"),
        state.config.default_model.as_deref().unwrap_or("<unset>"),
        state.config.theme,
        state.config.editor_mode.as_str(),
        state.config.fast_mode,
        state.config.copy_full_response,
        state.config.effort_level.as_deref().unwrap_or("auto"),
    );
    let _ = writeln!(
        &mut text,
        "\n[workspace]\nopenai_base_url={}\nopenai_headers={}\nopenai_query_params={}\nno_alt_screen={}\ntmux_golden_mode={}\nstatus_line_command={}\nstatus_line_padding={}",
        state.config.openai_base_url.as_deref().unwrap_or("<unset>"),
        serde_json::to_string(&json!(state.config.openai_headers)).unwrap_or_else(|_| "{}".to_string()),
        serde_json::to_string(&json!(state.config.openai_query_params)).unwrap_or_else(|_| "{}".to_string()),
        state.config.ui.no_alt_screen,
        state.config.ui.tmux_golden_mode,
        state
            .config
            .ui
            .status_line
            .as_ref()
            .map(|status_line| status_line.command.as_str())
            .unwrap_or("<unset>"),
        state
            .config
            .ui
            .status_line
            .as_ref()
            .map(|status_line| status_line.padding.to_string())
            .unwrap_or_else(|| "<unset>".to_string()),
    );
    let _ = writeln!(
        &mut text,
        "\n[session]\nprompt_color={}\nstatusline_enabled={}",
        state.prompt_color, state.statusline_enabled,
    );
    Ok(text.trim_end().to_string())
}

/// Renders the current permissions file summary without mutating transcript state.
pub(crate) fn render_permissions_panel(
    state: &AppState,
    resources: &LoadedResources,
) -> Result<String> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let permissions_path = paths.workspace_config_dir.join("permissions.toml");
    let settings = load_or_initialize_permissions(&permissions_path, resources)?;
    Ok(render_permissions_summary(&permissions_path, &settings))
}

/// Shows or materializes the workspace permissions file.
pub(crate) fn handle_permissions_command(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let permissions_path = paths.workspace_config_dir.join("permissions.toml");
    let mut settings = load_or_initialize_permissions(&permissions_path, resources)?;
    let trimmed = args.trim();
    match trimmed {
        "path" => {
            emit_system(
                state,
                session_store,
                format!("Permissions file: {}", permissions_path.display()),
            )
        }
        "" | "show" | "list" => emit_system(
            state,
            session_store,
            render_permissions_summary(&permissions_path, &settings),
        ),
        _ if trimmed.starts_with("allow ") => {
            let tool = trimmed.trim_start_matches("allow ").trim();
            if tool.is_empty() {
                return emit_system(
                    state,
                    session_store,
                    "Usage: /permissions allow <tool-id>".to_string(),
                );
            }
            if crate::tool_names::canonical_tool_name(tool) == "browser" {
                return emit_system(
                    state,
                    session_store,
                    browser_section_message(&permissions_path),
                );
            }
            set_permission_level(&mut settings, tool, "allow");
            write_permissions(&permissions_path, &settings)?;
            emit_system(
                state,
                session_store,
                format!("Set {} to `allow` in {}.", tool, permissions_path.display()),
            )
        }
        _ if trimmed.starts_with("deny ") => {
            let tool = trimmed.trim_start_matches("deny ").trim();
            if tool.is_empty() {
                return emit_system(
                    state,
                    session_store,
                    "Usage: /permissions deny <tool-id>".to_string(),
                );
            }
            if crate::tool_names::canonical_tool_name(tool) == "browser" {
                return emit_system(
                    state,
                    session_store,
                    browser_section_message(&permissions_path),
                );
            }
            set_permission_level(&mut settings, tool, "deny");
            write_permissions(&permissions_path, &settings)?;
            emit_system(
                state,
                session_store,
                format!("Set {} to `deny` in {}.", tool, permissions_path.display()),
            )
        }
        _ if trimmed.starts_with("ask ") => {
            let tool = trimmed.trim_start_matches("ask ").trim();
            if tool.is_empty() {
                return emit_system(
                    state,
                    session_store,
                    "Usage: /permissions ask <tool-id>".to_string(),
                );
            }
            if crate::tool_names::canonical_tool_name(tool) == "browser" {
                return emit_system(
                    state,
                    session_store,
                    browser_section_message(&permissions_path),
                );
            }
            set_permission_level(&mut settings, tool, "ask");
            write_permissions(&permissions_path, &settings)?;
            emit_system(
                state,
                session_store,
                format!("Set {} to `ask` in {}.", tool, permissions_path.display()),
            )
        }
        _ if trimmed.starts_with("remove ") => {
            let tool = trimmed.trim_start_matches("remove ").trim();
            if tool.is_empty() {
                return emit_system(
                    state,
                    session_store,
                    "Usage: /permissions remove <tool-id>".to_string(),
                );
            }
            if crate::tool_names::canonical_tool_name(tool) == "browser" {
                return emit_system(
                    state,
                    session_store,
                    browser_section_message(&permissions_path),
                );
            }
            settings.tools.remove(&permission_file_tool_key(tool));
            write_permissions(&permissions_path, &settings)?;
            emit_system(
                state,
                session_store,
                format!(
                    "Removed explicit rule for {} in {}.",
                    tool,
                    permissions_path.display()
                ),
            )
        }
        "summary" => describe_permissions(state, resources, session_store),
        _ => emit_system(
            state,
            session_store,
            "Usage: /permissions [show|list|path|summary|allow <tool-id>|deny <tool-id>|ask <tool-id>|remove <tool-id>]".to_string(),
        ),
    }
}

/// Shows or updates the workspace config file.
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
        return emit_system(state, session_store, render_config_summary(state)?);
    }

    if matches!(trimmed, "help" | "list") {
        return emit_system(state, session_store, render_supported_config_key_list());
    }

    if trimmed == "path" {
        return emit_system(
            state,
            session_store,
            format!(
                "User config path: {}\nWorkspace config path: {}",
                paths.user_config_file().display(),
                config_path.display()
            ),
        );
    }

    if trimmed == "open" {
        return emit_system(
            state,
            session_store,
            format!(
                "Open your user config file at {} or your workspace config file at {}.",
                paths.user_config_file().display(),
                config_path.display()
            ),
        );
    }

    let Some(rest) = trimmed.strip_prefix("set ") else {
        return emit_system(
            state,
            session_store,
            "Usage: /config [show|list|help|path|set <key> <value>]".to_string(),
        );
    };
    let Some((key, value)) = rest.split_once(' ') else {
        return emit_system(
            state,
            session_store,
            "Usage: /config set <key> <value>".to_string(),
        );
    };
    let parsed = match parse_config_cli_value(key, value.trim()) {
        Ok(value) => value,
        Err(error) => return emit_system(state, session_store, error.to_string()),
    };
    if let Err(error) = set_state_config_value(state, key, parsed) {
        return emit_system(state, session_store, error.to_string());
    }
    let path = match persist_config_setting(&paths, state, key) {
        Ok(path) => path,
        Err(error) => return emit_system(state, session_store, error.to_string()),
    };
    let scope = scope_label(config_setting_scope(key)?);
    emit_system(
        state,
        session_store,
        match path {
            Some(path) => format!("Updated {key} in {scope} config at {}.", path.display()),
            None => format!("Updated {key} for this session."),
        },
    )
}

/// Persists the current user-scoped settings to `~/.puffer/config.toml`.
pub(crate) fn persist_user_settings(state: &AppState) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    persist_user_config_settings(&paths, state)
}

/// Persists the currently selected provider and model to the user config file.
pub(crate) fn persist_user_model_selection(state: &AppState) -> Result<()> {
    persist_user_settings(state)
}

/// Reloads the layered Puffer config from disk into the active session state.
pub(crate) fn reload_config_from_disk(state: &mut AppState) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    state.config = load_config(&paths)?;
    state.vim_mode = state.config.editor_mode == "vim";
    state.fast_mode = state.config.fast_mode;
    state.effort_level = state
        .config
        .effort_level
        .clone()
        .unwrap_or_else(|| "auto".to_string());
    Ok(())
}

/// Shows or materializes the workspace keybindings file.
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

/// Shows or materializes the workspace hooks directory and example hook.
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
    let trimmed = args.trim();
    if trimmed == "path" {
        return emit_system(
            state,
            session_store,
            format!(
                "Hooks directory: {}\nSupported events: tool_start, tool_end, turn_end",
                hooks_dir.display()
            ),
        );
    }

    if trimmed == "list" {
        if resources.hooks.is_empty() {
            return emit_system(
                state,
                session_store,
                "No hook configurations are loaded.".to_string(),
            );
        }
        let mut summary = String::from("Loaded hooks:\n");
        for hook in &resources.hooks {
            let _ = writeln!(
                &mut summary,
                "- {} [{}] -> {}",
                hook.value.id, hook.value.event, hook.value.command
            );
        }
        return emit_system(state, session_store, summary);
    }

    if let Some(hook_id) = trimmed.strip_prefix("show ") {
        let hook_id = hook_id.trim();
        if hook_id.is_empty() {
            return emit_system(
                state,
                session_store,
                "Usage: /hooks show <hook-id>".to_string(),
            );
        }
        if let Some(hook) = hook_by_id(resources, hook_id) {
            return emit_system(
                state,
                session_store,
                format!(
                    "Hook {}\nevent={}\ncommand={}\nsource={}",
                    hook.value.id,
                    hook.value.event,
                    hook.value.command,
                    hook.source_info.path.display()
                ),
            );
        }
        return emit_system(state, session_store, format!("Unknown hook `{hook_id}`."));
    }

    if trimmed == "open" {
        return emit_system(
            state,
            session_store,
            format!("Open your hooks directory at {}.", hooks_dir.display()),
        );
    }

    emit_system(
        state,
        session_store,
        render_hooks_summary(state, resources)?,
    )
}

/// Renders the hooks summary shown by `/hooks` with no arguments.
pub(crate) fn render_hooks_summary(
    state: &AppState,
    resources: &LoadedResources,
) -> Result<String> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let hooks_dir = paths.workspace_config_dir.join("resources/hooks");
    fs::create_dir_all(&hooks_dir)?;
    let hooks_path = hooks_dir.join("tool_end.yaml");
    if !hooks_path.exists() {
        fs::write(&hooks_path, default_hooks_contents())?;
    }
    Ok(format!(
        "Hooks directory: {}\nloaded_hooks={}\nsupported_events=tool_start, tool_end, turn_end\n{}{}",
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
    ))
}

/// Builds the interactive `/hooks` action list used by the TUI picker.
pub(crate) fn render_hooks_actions(
    state: &AppState,
    resources: &LoadedResources,
) -> Result<Vec<CommandActionEntry>> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let hooks_dir = paths.workspace_config_dir.join("resources/hooks");
    fs::create_dir_all(&hooks_dir)?;
    let hooks_path = hooks_dir.join("tool_end.yaml");
    if !hooks_path.exists() {
        fs::write(&hooks_path, default_hooks_contents())?;
    }

    let mut actions = vec![
        CommandActionEntry {
            command: "/hooks open".to_string(),
            description: format!("Open workspace hooks directory ({})", hooks_dir.display()),
        },
        CommandActionEntry {
            command: "/hooks path".to_string(),
            description: "Show hook resource paths and supported events".to_string(),
        },
        CommandActionEntry {
            command: "/hooks list".to_string(),
            description: format!("List {} loaded hook(s)", resources.hooks.len()),
        },
    ];
    actions.extend(resources.hooks.iter().map(|hook| CommandActionEntry {
        command: format!("/hooks show {}", hook.value.id),
        description: format!(
            "{} [{}] {}",
            hook.value.id,
            hook.value.event,
            hook.source_info.path.display()
        ),
    }));
    Ok(actions)
}

fn set_permission_level(settings: &mut PermissionsSettings, tool: &str, level: &str) {
    if crate::tool_names::canonical_tool_name(tool) == "browser" {
        return;
    }
    settings
        .tools
        .insert(permission_file_tool_key(tool), level.to_string());
}

fn permission_file_tool_key(tool: &str) -> String {
    let normalized = normalize_tool_id(tool);
    let canonical = crate::tool_names::canonical_tool_name(tool);
    if canonical.is_empty() {
        return normalized;
    }
    if normalized.replace('_', "") == canonical {
        normalized
    } else {
        canonical
    }
}

fn render_permissions_summary(path: &PathBuf, settings: &PermissionsSettings) -> String {
    let mut body = String::from("Tool rules:\n");
    let visible_tools = settings
        .tools
        .iter()
        .filter(|(tool, _)| crate::tool_names::canonical_tool_name(tool) != "browser")
        .collect::<Vec<_>>();
    if visible_tools.is_empty() {
        body.push_str("- <none>\n");
    } else {
        for (tool, level) in visible_tools {
            let _ = writeln!(&mut body, "- {tool}: {level}");
        }
    }
    format!(
        "Permissions file: {}\n{}\n{}",
        path.display(),
        body.trim_end(),
        browser_section_message(path)
    )
}

fn browser_section_message(path: &PathBuf) -> String {
    format!(
        "Browser rules live under the `[browser]` section in {}.",
        path.display()
    )
}

/// Shows or updates the workspace sandbox configuration file.
pub(crate) fn handle_sandbox_command(
    state: &mut AppState,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let sandbox_path = paths.workspace_config_dir.join("sandbox.toml");
    let mut settings = load_or_initialize_sandbox_settings(&sandbox_path, state)?;
    let trimmed = args.trim();

    if trimmed.is_empty() || trimmed == "show" {
        return emit_system(
            state,
            session_store,
            render_sandbox_summary(&sandbox_path, &settings),
        );
    }

    if trimmed == "path" {
        return emit_system(
            state,
            session_store,
            format!("Sandbox config path: {}", sandbox_path.display()),
        );
    }

    if matches!(trimmed, "open" | "edit") {
        return open_sandbox_config(state, session_store, &sandbox_path);
    }

    if let Some(pattern) = trimmed.strip_prefix("exclude ") {
        let pattern = pattern.trim().trim_matches('"');
        if pattern.is_empty() {
            anyhow::bail!("expected a command pattern after `exclude`");
        }
        if !settings
            .excluded_commands
            .iter()
            .any(|existing| existing == pattern)
        {
            settings.excluded_commands.push(pattern.to_string());
        }
        write_sandbox_settings(&sandbox_path, &settings)?;
        return emit_system(
            state,
            session_store,
            format!(
                "Added sandbox exclusion `{pattern}` in {}.",
                sandbox_path.display()
            ),
        );
    }

    if trimmed == "clear-excludes" {
        settings.excluded_commands.clear();
        write_sandbox_settings(&sandbox_path, &settings)?;
        return emit_system(
            state,
            session_store,
            format!("Cleared sandbox exclusions in {}.", sandbox_path.display()),
        );
    }

    if let Some(value) = trimmed.strip_prefix("allow-unsandboxed ") {
        settings.allow_unsandboxed_fallback = parse_bool(value.trim())?;
        write_sandbox_settings(&sandbox_path, &settings)?;
        return emit_system(
            state,
            session_store,
            format!(
                "allow_unsandboxed_fallback set to {} in {}.",
                settings.allow_unsandboxed_fallback,
                sandbox_path.display()
            ),
        );
    }

    if let Some(value) = trimmed.strip_prefix("auto-allow ") {
        settings.auto_allow = parse_bool(value.trim())?;
        write_sandbox_settings(&sandbox_path, &settings)?;
        return emit_system(
            state,
            session_store,
            format!(
                "auto_allow set to {} in {}.",
                settings.auto_allow,
                sandbox_path.display()
            ),
        );
    }

    let mode = trimmed
        .strip_prefix("mode ")
        .map(str::trim)
        .unwrap_or(trimmed)
        .to_string();
    settings.mode = mode.clone();
    state.sandbox_mode = mode;
    write_sandbox_settings(&sandbox_path, &settings)?;
    emit_system(
        state,
        session_store,
        format!(
            "Sandbox mode set to {} in {}.",
            state.sandbox_mode,
            sandbox_path.display()
        ),
    )
}

/// Builds the interactive `/sandbox` action list used by the TUI picker.
pub(crate) fn render_sandbox_actions(state: &AppState) -> Result<Vec<CommandActionEntry>> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let sandbox_path = paths.workspace_config_dir.join("sandbox.toml");
    let settings = load_or_initialize_sandbox_settings(&sandbox_path, state)?;
    let mut actions = vec![
        CommandActionEntry {
            command: "/sandbox workspace-write".to_string(),
            description: sandbox_mode_description(&settings.mode, "workspace-write"),
        },
        CommandActionEntry {
            command: "/sandbox read-only".to_string(),
            description: sandbox_mode_description(&settings.mode, "read-only"),
        },
        CommandActionEntry {
            command: format!(
                "/sandbox auto-allow {}",
                if settings.auto_allow { "false" } else { "true" }
            ),
            description: format!(
                "Auto-allow tool prompts: {}",
                if settings.auto_allow { "on" } else { "off" }
            ),
        },
        CommandActionEntry {
            command: format!(
                "/sandbox allow-unsandboxed {}",
                if settings.allow_unsandboxed_fallback {
                    "false"
                } else {
                    "true"
                }
            ),
            description: format!(
                "Allow unsandboxed Bash fallback: {}",
                if settings.allow_unsandboxed_fallback {
                    "on"
                } else {
                    "off"
                }
            ),
        },
        CommandActionEntry {
            command: "/sandbox open".to_string(),
            description: format!("Open sandbox config ({})", sandbox_path.display()),
        },
        CommandActionEntry {
            command: "/sandbox path".to_string(),
            description: "Show sandbox config path".to_string(),
        },
    ];
    if !settings.excluded_commands.is_empty() {
        actions.push(CommandActionEntry {
            command: "/sandbox clear-excludes".to_string(),
            description: format!(
                "Clear {} excluded command pattern{}",
                settings.excluded_commands.len(),
                if settings.excluded_commands.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ),
        });
    }
    Ok(actions)
}

fn parse_bool(value: &str) -> Result<bool> {
    match value {
        "true" | "on" | "1" => Ok(true),
        "false" | "off" | "0" => Ok(false),
        _ => anyhow::bail!("expected a boolean value, got `{value}`"),
    }
}

fn open_sandbox_config(
    state: &mut AppState,
    session_store: &SessionStore,
    sandbox_path: &PathBuf,
) -> Result<()> {
    match open_text_file_in_editor(sandbox_path) {
        Ok(status) => emit_system(state, session_store, status),
        Err(error) => emit_system(
            state,
            session_store,
            format!(
                "Could not open sandbox config in an editor: {error}\nPath: {}",
                sandbox_path.display()
            ),
        ),
    }
}

fn default_keybindings_contents() -> &'static str {
    "submit = \"enter\"\nclear_input = \"esc\"\nexit = \"ctrl+c\"\n"
}

fn default_hooks_contents() -> &'static str {
    "id: tool-end\n\
event: tool_end\n\
command: echo \"$PUFFER_TOOL_ID:$PUFFER_TOOL_SUCCESS\"\n"
}

fn render_sandbox_summary(path: &PathBuf, settings: &SandboxSettings) -> String {
    let exclusions = if settings.excluded_commands.is_empty() {
        String::from("<none>")
    } else {
        settings.excluded_commands.join(", ")
    };
    format!(
        "Sandbox summary:\npath={}\nmode={}\nauto_allow={}\nallow_unsandboxed_fallback={}\nexcluded_commands={}",
        path.display(),
        settings.mode,
        settings.auto_allow,
        settings.allow_unsandboxed_fallback,
        exclusions
    )
}

fn sandbox_mode_description(current_mode: &str, candidate_mode: &str) -> String {
    if current_mode == candidate_mode {
        format!("Sandbox mode: {candidate_mode} (current)")
    } else {
        format!("Switch sandbox mode to {candidate_mode}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn permissions_round_trip_supports_allow_and_remove() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("permissions.toml");
        let mut settings = PermissionsSettings::default();
        set_permission_level(&mut settings, "read-file", "allow");
        write_permissions(&path, &settings).unwrap();
        let loaded: PermissionsSettings =
            toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.tools.get("read").map(String::as_str), Some("allow"));
    }

    #[test]
    fn browser_is_not_written_into_generic_permissions() {
        let mut settings = PermissionsSettings::default();
        set_permission_level(&mut settings, "browser", "allow");
        assert!(settings.tools.is_empty());
    }

    #[test]
    fn write_permissions_preserves_browser_section() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("permissions.toml");
        fs::write(
            &path,
            "[tools]\nread = \"ask\"\n\n[browser]\ndeny_domains = [\"example.com\"]\n",
        )
        .unwrap();

        let mut settings = PermissionsSettings::default();
        set_permission_level(&mut settings, "read", "allow");
        write_permissions(&path, &settings).unwrap();

        let stored = fs::read_to_string(&path).unwrap();
        assert!(stored.contains("[browser]"));
        assert!(stored.contains("example.com"));
    }
}
