mod markdown;
mod popup;
mod render;

use crate::popup::popup_rows;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use puffer_core::{
    dispatch_command, execute_user_turn, supported_commands, AppState, CommandSpec, MessageRole,
    ToolInvocation,
};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;
use puffer_session_store::{SessionStore, SessionSummary, TranscriptEvent};
use puffer_tools::{ToolInput, ToolRegistry};
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
    let mut tui = TuiState::default();
    let commands = supported_commands();

    loop {
        terminal.draw(|frame| {
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
                tui.overlay.as_ref(),
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
                        session_store,
                        &commands,
                        &mut tui,
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
    commands: &[CommandSpec],
    tui: &mut TuiState,
) -> Result<bool> {
    if tui.overlay.is_some() {
        return handle_overlay_key(
            key,
            state,
            resources,
            providers,
            auth_store,
            session_store,
            commands,
            tui,
        );
    }

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_exit = true;
            return Ok(true);
        }
        KeyCode::Esc => tui.clear(commands),
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
                tui.scroll_down(1, render::transcript_line_count(state));
            }
        }
        KeyCode::PageUp => tui.scroll_up(10),
        KeyCode::PageDown => tui.scroll_down(10, render::transcript_line_count(state)),
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
            if try_open_overlay(providers, session_store, tui, &current_input)? {
                return Ok(false);
            }
            let submitted = tui.take_input();
            handle_submit(
                state,
                resources,
                providers,
                auth_store,
                session_store,
                submitted,
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
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    session_store: &SessionStore,
    commands: &[CommandSpec],
    tui: &mut TuiState,
) -> Result<bool> {
    let Some(overlay) = tui.overlay.as_mut() else {
        return Ok(false);
    };

    match key.code {
        KeyCode::Esc => tui.overlay = None,
        KeyCode::Up => overlay.select_previous(),
        KeyCode::Down => overlay.select_next(),
        KeyCode::PageUp => overlay.page_up(),
        KeyCode::PageDown => overlay.page_down(),
        KeyCode::Enter => {
            if let Some(command) = overlay.selected_command() {
                tui.overlay = None;
                dispatch_command(
                    state,
                    commands,
                    resources,
                    providers,
                    auth_store,
                    session_store,
                    &command,
                )?;
            } else {
                tui.overlay = None;
            }
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_exit = true;
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

fn try_open_overlay(
    providers: &ProviderRegistry,
    session_store: &SessionStore,
    tui: &mut TuiState,
    submitted: &str,
) -> Result<bool> {
    let without_slash = submitted.trim_start_matches('/');
    let (name, args) = without_slash
        .split_once(' ')
        .map(|(name, args)| (name, args.trim()))
        .unwrap_or((without_slash, ""));

    let overlay = match name {
        "resume" | "continue" if args.is_empty() => {
            let sessions = session_store.list_sessions()?;
            if sessions.is_empty() {
                None
            } else {
                Some(OverlayState::SessionPicker {
                    sessions,
                    selection: 0,
                })
            }
        }
        "model" if args.is_empty() => {
            let entries = providers
                .models()
                .map(|model| ModelPickerEntry {
                    selector: format!("{}/{}", model.provider, model.id),
                    description: model.display_name.clone(),
                })
                .collect::<Vec<_>>();
            if entries.is_empty() {
                None
            } else {
                Some(OverlayState::ModelPicker {
                    entries,
                    selection: 0,
                })
            }
        }
        _ => None,
    };

    if let Some(overlay) = overlay {
        tui.overlay = Some(overlay);
        tui.input.clear();
        tui.cursor = 0;
        tui.slash_selection = 0;
        return Ok(true);
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

    Ok(())
}

fn append_tool_messages(
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
        format!("Tool {} [{}]\ninput: {}", invocation.tool_id, status, invocation.input)
    } else {
        format!(
            "Tool {} [{}]\ninput: {}\n{}",
            invocation.tool_id, status, invocation.input, output
        )
    }
}

fn execute_shell_shortcut(
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

fn parse_shell_shortcut(input: &str) -> Option<&str> {
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

#[derive(Debug, Clone, Default)]
struct TuiState {
    input: String,
    cursor: usize,
    slash_selection: usize,
    scroll_offset: u16,
    overlay: Option<OverlayState>,
}

impl TuiState {
    fn clear(&mut self, commands: &[CommandSpec]) {
        self.input.clear();
        self.cursor = 0;
        self.slash_selection = 0;
        self.sync(commands);
    }

    fn move_left(&mut self) {
        self.cursor = previous_boundary(&self.input, self.cursor);
    }

    fn move_right(&mut self) {
        self.cursor = next_boundary(&self.input, self.cursor);
    }

    fn move_home(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        self.cursor = self.input.len();
    }

    fn insert_char(&mut self, ch: char, commands: &[CommandSpec]) {
        self.input.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.sync(commands);
    }

    fn backspace(&mut self, commands: &[CommandSpec]) {
        if self.cursor == 0 {
            return;
        }
        let start = previous_boundary(&self.input, self.cursor);
        self.input.drain(start..self.cursor);
        self.cursor = start;
        self.sync(commands);
    }

    fn delete(&mut self, commands: &[CommandSpec]) {
        if self.cursor >= self.input.len() {
            return;
        }
        let end = next_boundary(&self.input, self.cursor);
        self.input.drain(self.cursor..end);
        self.sync(commands);
    }

    fn select_previous(&mut self, commands: &[CommandSpec]) {
        let count = self.matching_rows(commands).len();
        if count == 0 {
            return;
        }
        self.slash_selection = self.slash_selection.saturating_sub(1);
    }

    fn select_next(&mut self, commands: &[CommandSpec]) {
        let count = self.matching_rows(commands).len();
        if count == 0 {
            return;
        }
        self.slash_selection = (self.slash_selection + 1).min(count.saturating_sub(1));
    }

    fn apply_selected_command(&mut self, commands: &[CommandSpec]) -> bool {
        let rows = self.matching_rows(commands);
        let Some(command) = rows.get(self.slash_selection).copied() else {
            return false;
        };
        self.input = format!("/{}", command.name);
        if command.argument_hint.is_some() {
            self.input.push(' ');
        }
        self.cursor = self.input.len();
        self.sync(commands);
        true
    }

    fn complete_on_enter(&mut self, commands: &[CommandSpec]) -> bool {
        if !self.input.starts_with('/') || self.input.contains(' ') {
            return false;
        }
        let trimmed = self.input.trim_start_matches('/');
        if trimmed.is_empty() {
            return false;
        }
        let rows = self.matching_rows(commands);
        let Some(command) = rows.get(self.slash_selection).copied() else {
            return false;
        };
        if command.name == trimmed || command.aliases.contains(&trimmed) {
            return false;
        }
        self.apply_selected_command(commands)
    }

    fn take_input(&mut self) -> String {
        self.cursor = 0;
        self.slash_selection = 0;
        std::mem::take(&mut self.input)
    }

    fn matching_rows<'a>(&self, commands: &'a [CommandSpec]) -> Vec<&'a CommandSpec> {
        if self.input.starts_with('/') {
            popup_rows(&self.input, commands)
        } else {
            Vec::new()
        }
    }

    fn sync(&mut self, commands: &[CommandSpec]) {
        self.cursor = self.cursor.min(self.input.len());
        let count = self.matching_rows(commands).len();
        if count == 0 {
            self.slash_selection = 0;
        } else {
            self.slash_selection = self.slash_selection.min(count - 1);
        }
    }

    fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    fn scroll_down(&mut self, amount: u16, line_count: u16) {
        let max_offset = line_count.saturating_sub(1);
        self.scroll_offset = (self.scroll_offset + amount).min(max_offset);
    }
}

#[derive(Debug, Clone)]
pub(crate) enum OverlayState {
    SessionPicker {
        sessions: Vec<SessionSummary>,
        selection: usize,
    },
    ModelPicker {
        entries: Vec<ModelPickerEntry>,
        selection: usize,
    },
}

impl OverlayState {
    fn select_previous(&mut self) {
        match self {
            Self::SessionPicker { selection, .. } | Self::ModelPicker { selection, .. } => {
                *selection = selection.saturating_sub(1);
            }
        }
    }

    fn select_next(&mut self) {
        match self {
            Self::SessionPicker {
                sessions,
                selection,
            } => {
                *selection = (*selection + 1).min(sessions.len().saturating_sub(1));
            }
            Self::ModelPicker { entries, selection } => {
                *selection = (*selection + 1).min(entries.len().saturating_sub(1));
            }
        }
    }

    fn page_up(&mut self) {
        for _ in 0..10 {
            self.select_previous();
        }
    }

    fn page_down(&mut self) {
        for _ in 0..10 {
            self.select_next();
        }
    }

    fn selected_command(&self) -> Option<String> {
        match self {
            Self::SessionPicker {
                sessions,
                selection,
            } => sessions
                .get(*selection)
                .map(|session| format!("/resume {}", session.id)),
            Self::ModelPicker { entries, selection } => entries
                .get(*selection)
                .map(|entry| format!("/model {}", entry.selector)),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ModelPickerEntry {
    pub selector: String,
    pub description: String,
}

fn previous_boundary(input: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let mut index = cursor - 1;
    while index > 0 && !input.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn next_boundary(input: &str, cursor: usize) -> usize {
    if cursor >= input.len() {
        return input.len();
    }
    let mut index = cursor + 1;
    while index < input.len() && !input.is_char_boundary(index) {
        index += 1;
    }
    index.min(input.len())
}

#[cfg(test)]
mod tests;
