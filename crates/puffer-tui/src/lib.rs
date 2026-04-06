mod markdown;
mod flow;
#[path = "onboarding/mod.rs"]
mod onboarding;
mod popup;
mod render;
#[path = "state.rs"]
mod state;
mod usage;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size as terminal_size, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use puffer_core::{supported_commands, AppState, CommandSpec};
use puffer_provider_registry::{AuthStore, ProviderRegistry, StoredCredential};
use puffer_resources::LoadedResources;
use puffer_session_store::SessionStore;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use ratatui::{TerminalOptions, Viewport};
use std::io;
use std::io::IsTerminal;
use std::path::Path;
use std::time::Duration;

use state::TuiState;
pub(crate) use state::{AuthPickerAction, ModelPickerEntry, OverlayState};
use crate::flow::{
    allow_prompt_before_onboarding, apply_selected_provider, builtin_openai_base_url,
    builtin_openai_headers, builtin_openai_query_params, cancel_pending_submit,
    emit_system_message, handle_prompt_submit, handle_submit, persist_user_config,
    poll_pending_submit, run_embedded_auth_login, set_overlay_state, submit_next_queued_prompt,
    submit_queued_prompt_if_ready, try_open_overlay,
};

/// Runs the interactive Puffer TUI until the user exits.
pub fn run_app(
    state: &mut AppState,
    resources: &mut LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    session_store: &SessionStore,
    initial_prompt: Option<String>,
    no_alt_screen: bool,
) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        if let Some(prompt) = initial_prompt.filter(|prompt| !prompt.trim().is_empty()) {
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
            return Ok(());
        }
        anyhow::bail!("interactive TUI requires stdin and stdout to be terminals");
    }

    let bypass_onboarding = initial_prompt
        .as_deref()
        .map(allow_prompt_before_onboarding)
        .unwrap_or(false);
    enable_raw_mode()?;
    if !no_alt_screen {
        execute!(io::stdout(), EnterAlternateScreen)?;
    }

    let mut terminal = if no_alt_screen {
        let (_, height) = terminal_size()?;
        Terminal::with_options(
            CrosstermBackend::new(io::stdout()),
            TerminalOptions {
                viewport: Viewport::Inline(height.max(1)),
            },
        )?
    } else {
        Terminal::new(CrosstermBackend::new(io::stdout()))?
    };
    let mut tui = TuiState::default();
    tui.defer_prompt(
        initial_prompt
            .clone()
            .filter(|prompt| !prompt.trim().is_empty())
            .filter(|_| !bypass_onboarding),
    );
    let commands = supported_commands();
    if let Some(prompt) = initial_prompt.filter(|prompt| allow_prompt_before_onboarding(prompt)) {
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
    if !bypass_onboarding {
        submit_queued_prompt_if_ready(
            state,
            resources,
            providers,
            auth_store,
            auth_path,
            session_store,
            &mut tui,
            no_alt_screen,
        )?;
    }

    loop {
        if poll_pending_submit(state, auth_store, auth_path, session_store, &mut tui)? {
            submit_queued_prompt_if_ready(
                state,
                resources,
                providers,
                auth_store,
                auth_path,
                session_store,
                &mut tui,
                no_alt_screen,
            )?;
            submit_next_queued_prompt(
                state,
                resources,
                providers,
                auth_store,
                auth_path,
                session_store,
                &mut tui,
                no_alt_screen,
            )?;
        }
        terminal.draw(|frame| {
            render::set_active_overlay(tui.overlay.clone());
            render::set_pending_submit_state(
                tui.pending_submit
                    .as_ref()
                    .map(|pending| pending.prompt.clone()),
                tui.queued_prompts.iter().cloned().collect(),
            );
            render::render(
                frame,
                state,
                resources,
                providers,
                auth_store,
                &tui.input,
                tui.cursor,
                tui.slash_selection,
                tui.scroll_offset,
                &commands,
            )
        })?;
        if state.should_exit {
            break;
        }
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    if handle_key(
                        key,
                        state,
                        resources,
                        providers,
                        auth_store,
                        auth_path,
                        session_store,
                        &commands,
                        &mut tui,
                        no_alt_screen,
                    )? {
                        break;
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    if !no_alt_screen {
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    }
    Ok(())
}

fn handle_key(
    key: KeyEvent,
    state: &mut AppState,
    resources: &mut LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    session_store: &SessionStore,
    commands: &[CommandSpec],
    tui: &mut TuiState,
    no_alt_screen: bool,
) -> Result<bool> {
    if tui.overlay.is_some() {
        return handle_overlay_key(
            key,
            state,
            resources,
            providers,
            auth_store,
            auth_path,
            session_store,
            tui,
            no_alt_screen,
        );
    }

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_exit = true;
            return Ok(true);
        }
        KeyCode::Esc => {
            if cancel_pending_submit(state, session_store, tui)? {
                submit_next_queued_prompt(
                    state,
                    resources,
                    providers,
                    auth_store,
                    auth_path,
                    session_store,
                    tui,
                    no_alt_screen,
                )?;
            } else {
                tui.clear(commands);
            }
        }
        KeyCode::Left => tui.move_left(),
        KeyCode::Right => tui.move_right(),
        KeyCode::Home => tui.move_home(),
        KeyCode::End => tui.move_end(),
        KeyCode::Up => {
            if tui.input.starts_with('/') {
                tui.select_previous(commands)
            } else {
                tui.scroll_up(1);
            }
        }
        KeyCode::Down => {
            if tui.input.starts_with('/') {
                tui.select_next(commands)
            } else {
                tui.scroll_down(1, render::transcript_line_count(state, tui.has_pending_submit()));
            }
        }
        KeyCode::PageUp => {
            if tui.input.starts_with('/') {
                for _ in 0..10 {
                    tui.select_previous(commands);
                }
            } else {
                tui.scroll_up(10);
            }
        }
        KeyCode::PageDown => {
            if tui.input.starts_with('/') {
                for _ in 0..10 {
                    tui.select_next(commands);
                }
            } else {
                tui.scroll_down(
                    10,
                    render::transcript_line_count(state, tui.has_pending_submit()),
                );
            }
        }
        KeyCode::Backspace => tui.backspace(commands),
        KeyCode::Delete => tui.delete(commands),
        KeyCode::Tab => {
            let _ = tui.apply_selected_command(commands);
        }
        KeyCode::Enter => {
            if tui.complete_on_enter(commands) {
                return Ok(false);
            }
            let current_input = tui.input.clone();
            if try_open_overlay(
                state,
                providers,
                auth_store,
                session_store,
                tui,
                &current_input,
            )? {
                return Ok(false);
            }
            let submitted = tui.take_input();
            handle_prompt_submit(
                state,
                resources,
                providers,
                auth_store,
                auth_path,
                session_store,
                tui,
                submitted,
                no_alt_screen,
            )?;
            submit_queued_prompt_if_ready(
                state,
                resources,
                providers,
                auth_store,
                auth_path,
                session_store,
                tui,
                no_alt_screen,
            )?;
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            tui.insert_char(ch, commands)
        }
        _ => {}
    }
    Ok(false)
}

fn handle_overlay_key(
    key: KeyEvent,
    state: &mut AppState,
    resources: &mut LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    session_store: &SessionStore,
    tui: &mut TuiState,
    no_alt_screen: bool,
) -> Result<bool> {
    let Some(active_overlay) = tui.overlay.as_ref() else {
        return Ok(false);
    };

    if active_overlay.accepts_text_input() {
        match key.code {
            KeyCode::Esc => {
                let next = onboarding::back_overlay(active_overlay, providers, auth_store)?;
                set_overlay_state(tui, next);
            }
            KeyCode::Left => {
                if let Some(overlay) = tui.overlay.as_mut() {
                    overlay.move_left();
                }
            }
            KeyCode::Right => {
                if let Some(overlay) = tui.overlay.as_mut() {
                    overlay.move_right();
                }
            }
            KeyCode::Home => {
                if let Some(overlay) = tui.overlay.as_mut() {
                    overlay.move_home();
                }
            }
            KeyCode::End => {
                if let Some(overlay) = tui.overlay.as_mut() {
                    overlay.move_end();
                }
            }
            KeyCode::Backspace => {
                if let Some(overlay) = tui.overlay.as_mut() {
                    overlay.backspace();
                }
            }
            KeyCode::Delete => {
                if let Some(overlay) = tui.overlay.as_mut() {
                    overlay.delete();
                }
            }
            KeyCode::Enter => {
                let Some(provider_id) = active_overlay.selected_provider().map(str::to_string) else {
                    set_overlay_state(tui, None);
                    return Ok(false);
                };
                let key_value = active_overlay.api_key_value().unwrap_or("").trim().to_string();
                if key_value.is_empty() {
                    let next = onboarding::back_overlay(active_overlay, providers, auth_store)?;
                    set_overlay_state(tui, next);
                    emit_system_message(
                        state,
                        session_store,
                        format!("No API key was entered for {provider_id}."),
                    )?;
                    return Ok(false);
                }
                auth_store.set_api_key(provider_id.clone(), key_value);
                auth_store.save(auth_path)?;
                let next =
                    onboarding::provider_setup_overlay(providers, auth_store, &provider_id)?;
                set_overlay_state(tui, next);
                submit_queued_prompt_if_ready(
                    state,
                    resources,
                    providers,
                    auth_store,
                    auth_path,
                    session_store,
                    tui,
                    no_alt_screen,
                )?;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.should_exit = true;
                return Ok(true);
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(overlay) = tui.overlay.as_mut() {
                    overlay.insert_char(ch);
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    let overlay_snapshot = active_overlay.clone();
    match key.code {
        KeyCode::Esc => {
            set_overlay_state(
                tui,
                onboarding::back_overlay(&overlay_snapshot, providers, auth_store)?,
            );
        }
        KeyCode::Up => {
            if let Some(overlay) = tui.overlay.as_mut() {
                overlay.select_previous();
            }
        }
        KeyCode::Down => {
            if let Some(overlay) = tui.overlay.as_mut() {
                overlay.select_next();
            }
        }
        KeyCode::PageUp => {
            if let Some(overlay) = tui.overlay.as_mut() {
                overlay.page_up();
            }
        }
        KeyCode::PageDown => {
            if let Some(overlay) = tui.overlay.as_mut() {
                overlay.page_down();
            }
        }
        KeyCode::Backspace => {
            let commands = supported_commands();
            tui.backspace(&commands);
            if let Some(overlay) = tui.overlay.as_mut() {
                overlay.select_matching_query(&tui.input);
            }
        }
        KeyCode::Delete => {
            let commands = supported_commands();
            tui.delete(&commands);
            if let Some(overlay) = tui.overlay.as_mut() {
                overlay.select_matching_query(&tui.input);
            }
        }
        KeyCode::Enter => {
            if matches!(overlay_snapshot, OverlayState::ThemePicker { .. })
                && onboarding::initial_overlay(state, providers, auth_store)?.is_some()
            {
                let Some(command) = overlay_snapshot.selected_command() else {
                    set_overlay_state(tui, None);
                    return Ok(false);
                };
                handle_submit(
                    state,
                    resources,
                    providers,
                    auth_store,
                    auth_path,
                    session_store,
                    command,
                    no_alt_screen,
                )?;
                set_overlay_state(tui, onboarding::initial_provider_overlay(providers));
                submit_queued_prompt_if_ready(
                    state,
                    resources,
                    providers,
                    auth_store,
                    auth_path,
                    session_store,
                    tui,
                    no_alt_screen,
                )?;
                return Ok(false);
            }
            if let Some(provider_id) = overlay_snapshot.selected_provider().map(str::to_string) {
                match &overlay_snapshot {
                    OverlayState::ProviderPicker { .. } | OverlayState::LoginPicker { .. } => {
                        apply_selected_provider(state, &provider_id)?;
                        let next =
                            onboarding::provider_setup_overlay(providers, auth_store, &provider_id)?;
                        set_overlay_state(tui, next);
                    }
                    OverlayState::AuthPicker { .. } => {
                        let Some(action) = overlay_snapshot.selected_auth_action().cloned() else {
                            set_overlay_state(tui, None);
                            return Ok(false);
                        };
                        apply_selected_provider(state, &provider_id)?;
                        match action {
                            AuthPickerAction::OAuth => {
                                match run_embedded_auth_login(
                                    &provider_id,
                                    auth_store,
                                    auth_path,
                                    no_alt_screen,
                                ) {
                                    Ok(()) => {
                                        let next = onboarding::provider_setup_overlay(
                                            providers,
                                            auth_store,
                                            &provider_id,
                                        )?;
                                        set_overlay_state(tui, next);
                                    }
                                    Err(error) => {
                                        let next = onboarding::back_overlay(
                                            &overlay_snapshot,
                                            providers,
                                            auth_store,
                                        )?;
                                        set_overlay_state(tui, next);
                                        emit_system_message(
                                            state,
                                            session_store,
                                            format!("Login failed for {provider_id}: {error}"),
                                        )?;
                                    }
                                }
                            }
                            AuthPickerAction::ApiKey => {
                                set_overlay_state(
                                    tui,
                                    Some(OverlayState::ApiKeyPrompt {
                                        provider_id,
                                        value: String::new(),
                                        cursor: 0,
                                        onboarding: overlay_snapshot.is_onboarding(),
                                    }),
                                );
                            }
                            AuthPickerAction::Import(candidate) => {
                                let imported_openai_base_url = candidate.openai_base_url.clone();
                                let imported_openai_headers = candidate.openai_headers.clone();
                                let imported_openai_query_params =
                                    candidate.openai_query_params.clone();
                                match candidate.credential {
                                    StoredCredential::ApiKey { key } => {
                                        auth_store.set_api_key(provider_id.clone(), key);
                                    }
                                    StoredCredential::OAuth(credential) => {
                                        auth_store.set_oauth(provider_id.clone(), credential);
                                    }
                                }
                                if provider_id == "openai" {
                                    state.config.openai_base_url = imported_openai_base_url;
                                    state.config.openai_headers = imported_openai_headers;
                                    state.config.openai_query_params = imported_openai_query_params;
                                    persist_user_config(state)?;
                                    let base_url = state
                                        .config
                                        .openai_base_url
                                        .clone()
                                        .or_else(|| builtin_openai_base_url(resources));
                                    if let Some(base_url) = base_url {
                                        providers.set_openai_base_url(base_url);
                                    }
                                    let headers = if state.config.openai_headers.is_empty() {
                                        builtin_openai_headers(resources)
                                    } else {
                                        state
                                            .config
                                            .openai_headers
                                            .clone()
                                            .into_iter()
                                            .collect::<indexmap::IndexMap<_, _>>()
                                    };
                                    providers.set_openai_headers(headers);
                                    let query_params = if state.config.openai_query_params.is_empty()
                                    {
                                        builtin_openai_query_params(resources)
                                    } else {
                                        state
                                            .config
                                            .openai_query_params
                                            .clone()
                                            .into_iter()
                                            .collect::<indexmap::IndexMap<_, _>>()
                                    };
                                    providers.set_openai_query_params(query_params);
                                }
                                auth_store.save(auth_path)?;
                                let next = onboarding::provider_setup_overlay(
                                    providers,
                                    auth_store,
                                    &provider_id,
                                )?;
                                set_overlay_state(tui, next);
                            }
                            AuthPickerAction::UseStored | AuthPickerAction::NoneRequired => {
                                let next = onboarding::provider_setup_overlay(
                                    providers,
                                    auth_store,
                                    &provider_id,
                                )?;
                                set_overlay_state(tui, next);
                            }
                        }
                    }
                    OverlayState::ModelPicker {
                        onboarding: true, ..
                    } => {
                        let Some(model_id) = overlay_snapshot.selected_model().map(str::to_string) else {
                            set_overlay_state(tui, None);
                            return Ok(false);
                        };
                        state.current_provider = Some(provider_id.clone());
                        state.current_model = Some(format!("{provider_id}/{model_id}"));
                        state.config.default_provider = Some(provider_id.clone());
                        state.config.default_model = Some(format!("{provider_id}/{model_id}"));
                        persist_user_config(state)?;
                        emit_system_message(
                            state,
                            session_store,
                            format!("Active model set to {provider_id}/{model_id}."),
                        )?;
                        set_overlay_state(tui, None);
                    }
                    _ => {
                        if let Some(command) = overlay_snapshot.selected_command() {
                            set_overlay_state(tui, None);
                            handle_submit(
                                state,
                                resources,
                                providers,
                                auth_store,
                                auth_path,
                                session_store,
                                command,
                                no_alt_screen,
                            )?;
                        } else {
                            set_overlay_state(tui, None);
                        }
                    }
                }
            } else if let Some(command) = overlay_snapshot.selected_command() {
                set_overlay_state(tui, None);
                handle_submit(
                    state,
                    resources,
                    providers,
                    auth_store,
                    auth_path,
                    session_store,
                    command,
                    no_alt_screen,
                )?;
            } else {
                set_overlay_state(tui, None);
            }
            submit_queued_prompt_if_ready(
                state,
                resources,
                providers,
                auth_store,
                auth_path,
                session_store,
                tui,
                no_alt_screen,
            )?;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_exit = true;
            return Ok(true);
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if ch == '/' && tui.input.is_empty() {
                let commands = supported_commands();
                set_overlay_state(tui, None);
                tui.insert_char(ch, &commands);
                return Ok(false);
            }
            let commands = supported_commands();
            tui.insert_char(ch, &commands);
            if let Some(overlay) = tui.overlay.as_mut() {
                overlay.select_matching_query(&tui.input);
            }
        }
        _ => {}
    }
    Ok(false)
}

#[cfg(test)]
mod tests;
