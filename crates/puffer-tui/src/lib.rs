mod markdown;
mod popup;
mod render;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use puffer_core::{dispatch_command, execute_user_turn, supported_commands, AppState, MessageRole};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;
use puffer_session_store::{SessionStore, TranscriptEvent};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::time::Duration;

/// Runs the interactive Puffer TUI until the user exits.
pub fn run_app(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    session_store: &SessionStore,
    initial_prompt: Option<String>,
    no_alt_screen: bool,
) -> Result<()> {
    if let Some(prompt) = initial_prompt {
        handle_submit(
            state,
            resources,
            providers,
            auth_store,
            session_store,
            prompt,
        )?;
    }

    if !no_alt_screen {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
    }

    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut input = String::new();
    let commands = supported_commands();

    loop {
        terminal.draw(|frame| {
            render::render(frame, state, resources, providers, auth_store, &input, &commands)
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
                        session_store,
                        &mut input,
                    )? {
                        break;
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    if !no_alt_screen {
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    }
    Ok(())
}

fn handle_key(
    key: KeyEvent,
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    session_store: &SessionStore,
    input: &mut String,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_exit = true;
            return Ok(true);
        }
        KeyCode::Esc => input.clear(),
        KeyCode::Backspace => {
            input.pop();
        }
        KeyCode::Enter => {
            let submitted = std::mem::take(input);
            handle_submit(
                state,
                resources,
                providers,
                auth_store,
                session_store,
                submitted,
            )?;
        }
        KeyCode::Char(ch) => input.push(ch),
        _ => {}
    }
    Ok(false)
}

fn handle_submit(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    session_store: &SessionStore,
    submitted: String,
) -> Result<()> {
    let submitted = submitted.trim().to_string();
    if submitted.is_empty() {
        return Ok(());
    }

    if submitted.starts_with('/') {
        dispatch_command(
            state,
            &supported_commands(),
            resources,
            providers,
            auth_store,
            session_store,
            &submitted,
        )?;
        return Ok(());
    }

    state.push_message(MessageRole::User, submitted.clone());
    session_store.append_event(
        state.session.id,
        TranscriptEvent::UserMessage {
            text: submitted.clone(),
        },
    )?;

    match execute_user_turn(state, providers, auth_store, &submitted) {
        Ok(reply) => {
            state.push_message(MessageRole::Assistant, reply.clone());
            session_store.append_event(
                state.session.id,
                TranscriptEvent::AssistantMessage { text: reply },
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_config::PufferConfig;
    use puffer_session_store::SessionMetadata;
    use ratatui::backend::TestBackend;
    use uuid::Uuid;

    #[test]
    fn render_shows_command_popup_for_slash_input() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::new(
            PufferConfig::default(),
            std::env::current_dir().unwrap(),
            SessionMetadata {
                id: Uuid::nil(),
                display_name: None,
                cwd: std::env::current_dir().unwrap(),
                created_at_ms: 0,
                updated_at_ms: 0,
                parent_session_id: None,
                slug: None,
                tags: Vec::new(),
                note: None,
            },
        );
        terminal
            .draw(|frame| {
                render::render(
                    frame,
                    &state,
                    &LoadedResources::default(),
                    &ProviderRegistry::default(),
                    &AuthStore::default(),
                    "/rev",
                    &supported_commands(),
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("Inspector"));
        assert!(rendered.contains("/review"));
    }
}
