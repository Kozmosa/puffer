use anyhow::Result;
use puffer_config::{save_user_config, ConfigPaths};
use puffer_core::{run_resource_hooks, AppState, MessageRole, ToolInvocation};
use puffer_resources::LoadedResources;
use puffer_session_store::{SessionStore, TranscriptEvent};
use puffer_tools::{ToolInput, ToolRegistry};
use std::path::Path;

use crate::onboarding;
use crate::{render_tool_invocation, OverlayState, TuiState};
use puffer_provider_registry::{AuthStore, ProviderRegistry};

/// Persists the selected provider and clears any selected model until the user chooses one.
pub(crate) fn apply_selected_provider(
    state: &mut AppState,
    provider_id: &str,
) -> Result<()> {
    state.current_provider = Some(provider_id.to_string());
    state.current_model = None;
    state.config.default_provider = Some(provider_id.to_string());
    state.config.default_model = None;
    persist_user_config(state)
}

/// Persists the current user config to `~/.puffer/config.toml`.
pub(crate) fn persist_user_config(state: &AppState) -> Result<()> {
    let paths = ConfigPaths::discover(&state.cwd);
    save_user_config(&paths, &state.config)
}

/// Re-enters onboarding when needed or submits any queued prompt once setup is complete.
pub(crate) fn submit_queued_prompt_if_ready(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    session_store: &SessionStore,
    tui: &mut TuiState,
    no_alt_screen: bool,
    handle_submit: fn(
        &mut AppState,
        &LoadedResources,
        &mut ProviderRegistry,
        &mut AuthStore,
        &Path,
        &SessionStore,
        String,
        bool,
    ) -> Result<()>,
) -> Result<()> {
    if tui
        .deferred_prompt
        .as_deref()
        .map(str::trim)
        .is_some_and(|prompt| prompt == "/help" || prompt == "/?")
    {
        if let Some(prompt) = tui.take_deferred_prompt() {
            handle_submit(
                state,
                resources,
                providers,
                auth_store,
                auth_path,
                session_store,
                prompt,
                no_alt_screen,
            )?;
        }
        return Ok(());
    }
    if tui.overlay.is_some() {
        return Ok(());
    }
    if let Some(overlay) = onboarding::initial_overlay(state, providers, auth_store)? {
        tui.overlay = Some(overlay);
        return Ok(());
    }
    if let Some(prompt) = tui.take_deferred_prompt() {
        handle_submit(
            state,
            resources,
            providers,
            auth_store,
            auth_path,
            session_store,
            prompt,
            no_alt_screen,
        )?;
    }
    Ok(())
}

/// Appends one system message to the in-memory transcript and persisted session log.
pub(crate) fn emit_system_message(
    state: &mut AppState,
    session_store: &SessionStore,
    text: String,
) -> Result<()> {
    state.push_message(MessageRole::System, text.clone());
    session_store.append_event(state.session.id, TranscriptEvent::SystemMessage { text })?;
    Ok(())
}

/// Returns true when the active selection belongs to the provider being logged out.
pub(crate) fn active_selection_uses_provider(
    state: &AppState,
    provider_id: &str,
) -> bool {
    if state.current_provider.as_deref() == Some(provider_id) {
        return true;
    }
    state.current_model
        .as_deref()
        .and_then(|selector| selector.split_once('/'))
        .map(|(provider, _)| provider == provider_id)
        .unwrap_or(false)
}

/// Parses the `!cmd` shell shortcut form used by Claude/Codex-style CLIs.
pub(crate) fn parse_shell_shortcut(input: &str) -> Option<&str> {
    let command = input
        .strip_prefix("!!")
        .or_else(|| input.strip_prefix('!'))?;
    let trimmed = command.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Returns true for slash commands that should bypass startup onboarding.
pub(crate) fn allow_prompt_before_onboarding(prompt: &str) -> bool {
    matches!(
        prompt.trim(),
        "/help" | "/theme" | "/doctor" | "/status" | "/usage"
    )
}

/// Replaces the active overlay and clears any overlay query text.
pub(crate) fn set_overlay_state(
    tui: &mut TuiState,
    overlay: Option<OverlayState>,
) {
    tui.overlay = overlay;
    tui.input.clear();
    tui.cursor = 0;
    tui.slash_selection = 0;
}

/// Records tool invocations into transcript/task/session state.
pub(crate) fn append_tool_messages(
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
        let rendered = render_tool_invocation(invocation);
        state.push_message(MessageRole::System, rendered.clone());
        session_store.append_event(
            state.session.id,
            TranscriptEvent::SystemMessage { text: rendered },
        )?;
    }
    Ok(())
}

/// Executes a `!cmd` shell shortcut and records the result into the transcript.
pub(crate) fn execute_shell_shortcut(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    shell_command: &str,
) -> Result<()> {
    let rendered_command = format!("!{shell_command}");
    state.push_message(MessageRole::User, rendered_command.clone());
    session_store.append_event(
        state.session.id,
        TranscriptEvent::UserMessage {
            text: rendered_command,
        },
    )?;

    let registry = ToolRegistry::from_resources(resources);
    let result = registry.execute(
        "bash",
        &state.cwd,
        ToolInput::Bash {
            command: shell_command.to_string(),
        },
    )?;
    state.record_task("bash", shell_command.to_string(), result.success);
    run_resource_hooks(
        resources,
        &state.cwd,
        "tool_end",
        &[
            ("PUFFER_TOOL_ID", "bash".to_string()),
            (
                "PUFFER_TOOL_INPUT",
                format!("{{\"command\":\"{}\"}}", shell_command.replace('"', "\\\"")),
            ),
            (
                "PUFFER_TOOL_SUCCESS",
                if result.success { "true" } else { "false" }.to_string(),
            ),
            ("PUFFER_TOOL_STDOUT", result.output.stdout.clone()),
            ("PUFFER_TOOL_STDERR", result.output.stderr.clone()),
        ],
    );

    let reply = if result.output.stderr.is_empty() {
        result.output.stdout
    } else if result.output.stdout.is_empty() {
        result.output.stderr
    } else {
        format!("{}\n{}", result.output.stdout, result.output.stderr)
    };
    let role = if result.success {
        MessageRole::Assistant
    } else {
        MessageRole::System
    };
    state.push_message(role, reply.clone());
    session_store.append_event(
        state.session.id,
        TranscriptEvent::AssistantMessage { text: reply },
    )?;
    Ok(())
}
