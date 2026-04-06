use anyhow::Result;
use puffer_config::{save_user_config, ConfigPaths};
use puffer_core::{
    dispatch_command, execute_user_turn, execute_user_turn_streaming, reload_runtime_resources,
    run_resource_hooks, supported_commands, AppState, MessageRole, ToolInvocation,
    TurnStreamEvent,
};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;
use puffer_session_store::{SessionStore, TranscriptEvent};
use puffer_tools::{ToolInput, ToolRegistry};
use std::io;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;

use crate::onboarding;
use crate::state::{PendingSubmit, PendingSubmitEvent, PendingSubmitResult};
use crate::{OverlayState, TuiState};

/// Opens a TUI overlay for slash commands that map to picker UI.
pub(crate) fn try_open_overlay(
    state: &AppState,
    providers: &mut ProviderRegistry,
    auth_store: &AuthStore,
    session_store: &SessionStore,
    tui: &mut TuiState,
    submitted: &str,
) -> Result<bool> {
    if let Some(overlay) =
        onboarding::overlay_from_command(state, providers, auth_store, session_store, submitted)?
    {
        set_overlay_state(tui, Some(overlay));
        return Ok(true);
    }
    Ok(false)
}

/// Replaces the active overlay and clears the overlay query buffer.
pub(crate) fn set_overlay_state(tui: &mut TuiState, overlay: Option<OverlayState>) {
    tui.overlay = overlay;
    tui.input.clear();
    tui.cursor = 0;
    tui.slash_selection = 0;
}

/// Returns true when the submitted text should run through the async provider path.
pub(crate) fn is_provider_prompt_input(submitted: &str) -> bool {
    let submitted = submitted.trim();
    !submitted.is_empty()
        && !submitted.starts_with('/')
        && parse_shell_shortcut(submitted).is_none()
        && !is_auth_command_input(submitted)
}

/// Handles one prompt submission from the interactive composer.
pub(crate) fn handle_prompt_submit(
    state: &mut AppState,
    resources: &mut LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    session_store: &SessionStore,
    tui: &mut TuiState,
    submitted: String,
    no_alt_screen: bool,
) -> Result<()> {
    let submitted = submitted.trim().to_string();
    if submitted.is_empty() {
        return Ok(());
    }
    if tui.has_pending_submit() && is_provider_prompt_input(&submitted) {
        tui.enqueue_prompt(submitted);
        return Ok(());
    }
    if !is_provider_prompt_input(&submitted) {
        return handle_submit(
            state,
            resources,
            providers,
            auth_store,
            auth_path,
            session_store,
            submitted,
            no_alt_screen,
        );
    }
    if tui.has_pending_submit() {
        return Ok(());
    }

    state.push_message(MessageRole::User, submitted.clone());
    session_store.append_event(
        state.session.id,
        TranscriptEvent::UserMessage {
            text: submitted.clone(),
        },
    )?;

    let worker_state = state.clone();
    let worker_resources = resources.clone();
    let worker_providers = providers.clone();
    let worker_prompt = submitted.clone();
    let mut worker_auth_store = auth_store.clone();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let event_sender = sender.clone();
        let outcome = execute_user_turn_streaming(
            &worker_state,
            &worker_resources,
            &worker_providers,
            &mut worker_auth_store,
            &worker_prompt,
            |event| match event {
                TurnStreamEvent::TextDelta(delta) => {
                    let _ = event_sender.send(PendingSubmitEvent::TextDelta(delta));
                }
                TurnStreamEvent::ToolInvocations(invocations) => {
                    let _ = event_sender.send(PendingSubmitEvent::ToolInvocations(invocations));
                }
            },
        )
        .map_err(|error| error.to_string());
        let _ = sender.send(PendingSubmitEvent::Finished(PendingSubmitResult {
            outcome,
            auth_store: worker_auth_store,
        }));
    });
    tui.pending_submit = Some(PendingSubmit {
        prompt: submitted,
        receiver,
        rendered_tool_invocations: 0,
    });
    Ok(())
}

/// Cancels the in-flight provider turn and discards future worker output.
pub(crate) fn cancel_pending_submit(
    state: &mut AppState,
    session_store: &SessionStore,
    tui: &mut TuiState,
) -> Result<bool> {
    if tui.pending_submit.take().is_none() {
        return Ok(false);
    }
    let message = "Interrupted by user.".to_string();
    state.push_message(MessageRole::System, message.clone());
    session_store.append_event(
        state.session.id,
        TranscriptEvent::SystemMessage { text: message },
    )?;
    Ok(true)
}

/// Starts the next queued prompt when no turn is currently running.
pub(crate) fn submit_next_queued_prompt(
    state: &mut AppState,
    resources: &mut LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    session_store: &SessionStore,
    tui: &mut TuiState,
    no_alt_screen: bool,
) -> Result<bool> {
    let Some(prompt) = tui.dequeue_prompt() else {
        return Ok(false);
    };
    handle_prompt_submit(
        state,
        resources,
        providers,
        auth_store,
        auth_path,
        session_store,
        tui,
        prompt,
        no_alt_screen,
    )?;
    Ok(true)
}

/// Applies any completed async provider turn to session and transcript state.
pub(crate) fn poll_pending_submit(
    state: &mut AppState,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    session_store: &SessionStore,
    tui: &mut TuiState,
) -> Result<bool> {
    let Some(pending) = tui.pending_submit.as_mut() else {
        return Ok(false);
    };
    let mut completed = false;
    loop {
        let event = match pending.receiver.try_recv() {
            Ok(event) => event,
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => PendingSubmitEvent::Finished(PendingSubmitResult {
                outcome: Err("background request disconnected".to_string()),
                auth_store: auth_store.clone(),
            }),
        };
        match event {
            PendingSubmitEvent::TextDelta(delta) => append_assistant_delta(state, &delta),
            PendingSubmitEvent::ToolInvocations(invocations) => {
                pending.rendered_tool_invocations += invocations.len();
                append_tool_messages(state, session_store, &invocations)?;
            }
            PendingSubmitEvent::Finished(result) => {
                completed = true;
                let rendered_tool_invocations = pending.rendered_tool_invocations;
                let previous_auth_store = auth_store.clone();
                *auth_store = result.auth_store;
                match result.outcome {
                    Ok(turn) => {
                        if rendered_tool_invocations < turn.tool_invocations.len() {
                            append_tool_messages(
                                state,
                                session_store,
                                &turn.tool_invocations[rendered_tool_invocations..],
                            )?;
                        }
                        finalize_assistant_text(state, session_store, &turn.assistant_text)?;
                    }
                    Err(error) => {
                        let message = format!("Provider request failed: {error}");
                        state.push_message(MessageRole::System, message.clone());
                        session_store.append_event(
                            state.session.id,
                            TranscriptEvent::SystemMessage { text: message },
                        )?;
                    }
                }
                if *auth_store != previous_auth_store {
                    auth_store.save(auth_path)?;
                }
                break;
            }
        }
    }
    if completed {
        tui.pending_submit = None;
    }
    Ok(completed)
}

/// Submits prompt/auth/shell input from the TUI prompt.
pub(crate) fn handle_submit(
    state: &mut AppState,
    resources: &mut LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    session_store: &SessionStore,
    submitted: String,
    no_alt_screen: bool,
) -> Result<()> {
    let submitted = submitted.trim().to_string();
    if submitted.is_empty() {
        return Ok(());
    }

    if handle_auth_command(
        state,
        auth_store,
        auth_path,
        session_store,
        &submitted,
        no_alt_screen,
    )? {
        return Ok(());
    }

    if submitted.starts_with('/') {
        let previous_auth_store = auth_store.clone();
        dispatch_command(
            state,
            &supported_commands(),
            resources,
            providers,
            auth_store,
            session_store,
            &submitted,
        )?;
        if *auth_store != previous_auth_store {
            auth_store.save(auth_path)?;
        }
        maybe_apply_requested_reload(state, resources, providers, auth_store, session_store)?;
        return Ok(());
    }

    if let Some(shell_command) = parse_shell_shortcut(&submitted) {
        execute_shell_shortcut(state, resources, session_store, shell_command)?;
        return Ok(());
    }

    state.push_message(MessageRole::User, submitted.clone());
    session_store.append_event(
        state.session.id,
        TranscriptEvent::UserMessage {
            text: submitted.clone(),
        },
    )?;

    let previous_auth_store = auth_store.clone();
    match execute_user_turn(state, resources, providers, auth_store, &submitted) {
        Ok(turn) => {
            append_tool_messages(state, session_store, &turn.tool_invocations)?;
            state.push_message(MessageRole::Assistant, turn.assistant_text.clone());
            session_store.append_event(
                state.session.id,
                TranscriptEvent::AssistantMessage {
                    text: turn.assistant_text,
                },
            )?;
        }
        Err(error) => {
            let message = format!("Provider request failed: {error}");
            state.push_message(MessageRole::System, message.clone());
            session_store.append_event(
                state.session.id,
                TranscriptEvent::SystemMessage { text: message },
            )?;
        }
    }
    if *auth_store != previous_auth_store {
        auth_store.save(auth_path)?;
    }

    Ok(())
}

fn is_auth_command_input(submitted: &str) -> bool {
    matches!(submit_command_name(submitted), "login" | "logout")
}

fn append_assistant_delta(state: &mut AppState, delta: &str) {
    if delta.is_empty() {
        return;
    }
    if let Some(last) = state.transcript.last_mut() {
        if last.role == MessageRole::Assistant {
            last.text.push_str(delta);
            return;
        }
    }
    state.push_message(MessageRole::Assistant, delta.to_string());
}

fn finalize_assistant_text(
    state: &mut AppState,
    session_store: &SessionStore,
    assistant_text: &str,
) -> Result<()> {
    if let Some(last) = state.transcript.last_mut() {
        if last.role == MessageRole::Assistant {
            last.text = assistant_text.to_string();
        } else {
            state.push_message(MessageRole::Assistant, assistant_text.to_string());
        }
    } else {
        state.push_message(MessageRole::Assistant, assistant_text.to_string());
    }
    session_store.append_event(
        state.session.id,
        TranscriptEvent::AssistantMessage {
            text: assistant_text.to_string(),
        },
    )?;
    Ok(())
}

fn submit_command_name(submitted: &str) -> &str {
    submitted
        .trim()
        .trim_start_matches('/')
        .split_once(' ')
        .map(|(name, _)| name)
        .unwrap_or_else(|| submitted.trim().trim_start_matches('/'))
}

/// Persists the selected provider and clears any selected model until the user chooses one.
pub(crate) fn apply_selected_provider(state: &mut AppState, provider_id: &str) -> Result<()> {
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

/// Returns the builtin OpenAI base URL from loaded provider resources.
pub(crate) fn builtin_openai_base_url(resources: &LoadedResources) -> Option<String> {
    resources
        .providers
        .iter()
        .find(|provider| provider.value.id == "openai")
        .map(|provider| provider.value.base_url.clone())
}

/// Returns builtin OpenAI headers from loaded provider resources.
pub(crate) fn builtin_openai_headers(
    resources: &LoadedResources,
) -> indexmap::IndexMap<String, String> {
    resources
        .providers
        .iter()
        .find(|provider| provider.value.id == "openai")
        .map(|provider| provider.value.headers.clone())
        .unwrap_or_default()
}

/// Returns builtin OpenAI query params from loaded provider resources.
pub(crate) fn builtin_openai_query_params(
    resources: &LoadedResources,
) -> indexmap::IndexMap<String, String> {
    resources
        .providers
        .iter()
        .find(|provider| provider.value.id == "openai")
        .map(|provider| provider.value.query_params.clone())
        .unwrap_or_default()
}

/// Re-enters onboarding when needed or submits any queued prompt once setup is complete.
pub(crate) fn submit_queued_prompt_if_ready(
    state: &mut AppState,
    resources: &mut LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    session_store: &SessionStore,
    tui: &mut TuiState,
    no_alt_screen: bool,
) -> Result<()> {
    if tui.has_pending_submit() {
        return Ok(());
    }
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
        handle_prompt_submit(
            state,
            resources,
            providers,
            auth_store,
            auth_path,
            session_store,
            tui,
            prompt,
            no_alt_screen,
        )?;
    }
    Ok(())
}

/// Reloads runtime resources when commands request it and emits the reload summary.
pub(crate) fn maybe_apply_requested_reload(
    state: &mut AppState,
    resources: &mut LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &AuthStore,
    session_store: &SessionStore,
) -> Result<()> {
    if !state.reload_resources_requested {
        return Ok(());
    }
    state.reload_resources_requested = false;
    let summary = reload_runtime_resources(state, resources, providers, auth_store)?;
    emit_system_message(state, session_store, summary)
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

/// Handles embedded login/logout commands from the TUI.
pub(crate) fn handle_auth_command(
    state: &mut AppState,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    session_store: &SessionStore,
    submitted: &str,
    no_alt_screen: bool,
) -> Result<bool> {
    let without_slash = submitted.trim_start_matches('/');
    let (name, args) = without_slash
        .split_once(' ')
        .map(|(name, args)| (name, args.trim()))
        .unwrap_or((without_slash, ""));
    if name == "login" {
        let provider = if args.is_empty() {
            state.current_provider.as_deref().unwrap_or("anthropic")
        } else {
            args
        };
        run_embedded_auth_login(provider, auth_store, auth_path, no_alt_screen)?;
        let message = format!("Completed login flow for {provider}.");
        state.push_message(MessageRole::System, message.clone());
        session_store.append_event(
            state.session.id,
            TranscriptEvent::SystemMessage { text: message },
        )?;
        return Ok(true);
    }

    if name != "logout" {
        return Ok(false);
    }

    let provider = if args.is_empty() {
        state.current_provider.as_deref().unwrap_or("anthropic")
    } else {
        args
    }
    .to_string();
    let removed = auth_store.remove(&provider);
    let cleared_active_provider = active_selection_uses_provider(state, provider.as_str());
    if cleared_active_provider {
        state.current_provider = None;
        state.current_model = None;
        state.config.default_provider = None;
        state.config.default_model = None;
        persist_user_config(state)?;
    }
    let message = if removed.is_some() {
        auth_store.save(auth_path)?;
        if cleared_active_provider {
            format!("Removed stored credentials for {provider} and cleared the active selection.")
        } else {
            format!("Removed stored credentials for {provider}.")
        }
    } else if cleared_active_provider {
        format!("No stored credentials exist for {provider}; cleared the active selection.")
    } else {
        format!("No stored credentials exist for {provider}.")
    };
    state.push_message(MessageRole::System, message.clone());
    session_store.append_event(
        state.session.id,
        TranscriptEvent::SystemMessage { text: message },
    )?;
    Ok(true)
}

/// Returns true when the active selection belongs to the provider being logged out.
pub(crate) fn active_selection_uses_provider(state: &AppState, provider_id: &str) -> bool {
    if state.current_provider.as_deref() == Some(provider_id) {
        return true;
    }
    state.current_model
        .as_deref()
        .and_then(|selector| selector.split_once('/'))
        .map(|(provider, _)| provider == provider_id)
        .unwrap_or(false)
}

pub(crate) fn run_embedded_auth_login(
    provider: &str,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    no_alt_screen: bool,
) -> Result<()> {
    if !no_alt_screen {
        crossterm::terminal::disable_raw_mode()?;
        crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
    }

    let status = Command::new(std::env::current_exe()?)
        .arg("auth")
        .arg("login")
        .arg(provider)
        .status()?;

    if !no_alt_screen {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    }

    if !status.success() {
        anyhow::bail!("login flow for {provider} exited with {}", status);
    }

    *auth_store = AuthStore::load(auth_path)?;
    Ok(())
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

fn render_tool_invocation(invocation: &ToolInvocation) -> String {
    let status = if invocation.success { "ok" } else { "error" };
    let output = invocation.output.trim();
    if output.is_empty() {
        format!(
            "Tool {} [{}]\ninput: {}",
            invocation.tool_id, status, invocation.input
        )
    } else {
        format!(
            "Tool {} [{}]\ninput: {}\n{}",
            invocation.tool_id, status, invocation.input, output
        )
    }
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
            timeout: None,
            run_in_background: false,
            dangerously_disable_sandbox: false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_config::{ensure_workspace_dirs, ConfigPaths, PufferConfig};
    use puffer_session_store::SessionMetadata;
    use tempfile::tempdir;

    fn sample_state(session: SessionMetadata, cwd: &Path) -> AppState {
        AppState::new(PufferConfig::default(), cwd.to_path_buf(), session)
    }

    #[test]
    fn provider_prompt_detection_matches_interactive_surface() {
        assert!(is_provider_prompt_input("henlo"));
        assert!(is_provider_prompt_input(" review this diff "));
        assert!(!is_provider_prompt_input(""));
        assert!(!is_provider_prompt_input("/help"));
        assert!(!is_provider_prompt_input("!pwd"));
        assert!(!is_provider_prompt_input("login openai"));
        assert!(!is_provider_prompt_input("/logout"));
    }

    #[test]
    fn handle_prompt_submit_starts_async_provider_turn_and_polls_result() {
        let tempdir = tempdir().unwrap();
        let paths = ConfigPaths::discover(tempdir.path());
        ensure_workspace_dirs(&paths).unwrap();
        let session_store = SessionStore::from_paths(&paths).unwrap();
        let session = session_store.create_session(tempdir.path().to_path_buf()).unwrap();
        let mut state = sample_state(session, tempdir.path());
        let mut resources = LoadedResources::default();
        let mut providers = ProviderRegistry::new();
        let auth_path = paths.user_config_dir.join("auth.json");
        let mut auth_store = AuthStore::default();
        let mut tui = TuiState::default();

        handle_prompt_submit(
            &mut state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &auth_path,
            &session_store,
            &mut tui,
            "henlo".to_string(),
            true,
        )
        .unwrap();

        assert!(tui.has_pending_submit());
        assert!(matches!(state.transcript.first(), Some(message) if message.text == "henlo"));

        let mut completed = false;
        for _ in 0..20 {
            if poll_pending_submit(
                &mut state,
                &mut auth_store,
                &auth_path,
                &session_store,
                &mut tui,
            )
            .unwrap()
            {
                completed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(completed);
        assert!(!tui.has_pending_submit());
        assert!(state.transcript.iter().any(|message| {
            message.role == MessageRole::System
                && message.text.starts_with("Provider request failed:")
        }));
    }

    #[test]
    fn handle_prompt_submit_queues_prompt_while_turn_is_running() {
        let tempdir = tempdir().unwrap();
        let paths = ConfigPaths::discover(tempdir.path());
        ensure_workspace_dirs(&paths).unwrap();
        let session_store = SessionStore::from_paths(&paths).unwrap();
        let session = session_store.create_session(tempdir.path().to_path_buf()).unwrap();
        let mut state = sample_state(session, tempdir.path());
        let mut resources = LoadedResources::default();
        let mut providers = ProviderRegistry::new();
        let auth_path = paths.user_config_dir.join("auth.json");
        let mut auth_store = AuthStore::default();
        let mut tui = TuiState::default();

        handle_prompt_submit(
            &mut state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &auth_path,
            &session_store,
            &mut tui,
            "first".to_string(),
            true,
        )
        .unwrap();
        handle_prompt_submit(
            &mut state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &auth_path,
            &session_store,
            &mut tui,
            "second".to_string(),
            true,
        )
        .unwrap();

        assert!(tui.has_pending_submit());
        assert_eq!(tui.queued_prompts.len(), 1);
        assert_eq!(tui.queued_prompts.front().map(String::as_str), Some("second"));
        assert!(matches!(state.transcript.first(), Some(message) if message.text == "first"));
    }

    #[test]
    fn cancel_pending_submit_records_interrupt_and_starts_next_queued_prompt() {
        let tempdir = tempdir().unwrap();
        let paths = ConfigPaths::discover(tempdir.path());
        ensure_workspace_dirs(&paths).unwrap();
        let session_store = SessionStore::from_paths(&paths).unwrap();
        let session = session_store.create_session(tempdir.path().to_path_buf()).unwrap();
        let mut state = sample_state(session, tempdir.path());
        let mut resources = LoadedResources::default();
        let mut providers = ProviderRegistry::new();
        let auth_path = paths.user_config_dir.join("auth.json");
        let mut auth_store = AuthStore::default();
        let mut tui = TuiState::default();

        handle_prompt_submit(
            &mut state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &auth_path,
            &session_store,
            &mut tui,
            "first".to_string(),
            true,
        )
        .unwrap();
        handle_prompt_submit(
            &mut state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &auth_path,
            &session_store,
            &mut tui,
            "second".to_string(),
            true,
        )
        .unwrap();

        assert!(cancel_pending_submit(&mut state, &session_store, &mut tui).unwrap());
        assert!(!tui.has_pending_submit());
        assert!(state.transcript.iter().any(|message| {
            message.role == MessageRole::System && message.text == "Interrupted by user."
        }));

        assert!(
            submit_next_queued_prompt(
                &mut state,
                &mut resources,
                &mut providers,
                &mut auth_store,
                &auth_path,
                &session_store,
                &mut tui,
                true,
        )
        .unwrap()
        );
        assert!(tui.has_pending_submit());
        assert!(tui.queued_prompts.is_empty());
        assert!(state.transcript.iter().any(|message| {
            message.role == MessageRole::User && message.text == "second"
        }));
    }
}
