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
use puffer_config::{ensure_workspace_dirs, ConfigPaths};
use puffer_core::{
    dispatch_command, execute_user_turn, run_resource_hooks, supported_commands, AppState,
    CommandSpec, MessageRole, ToolInvocation,
};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;
use puffer_session_store::{SessionStore, SessionSummary, TranscriptEvent};
use puffer_tools::{ToolInput, ToolRegistry};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use serde::Deserialize;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Runs the interactive Puffer TUI until the user exits.
pub fn run_app(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
    auth_path: &Path,
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
            auth_path,
            session_store,
            prompt,
            no_alt_screen,
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
            render::set_active_overlay(tui.overlay.clone());
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
            if try_open_overlay(state, providers, auth_store, session_store, tui, &current_input)?
            {
                return Ok(false);
            }
            let submitted = tui.take_input();
            handle_submit(
                state,
                resources,
                providers,
                auth_store,
                auth_path,
                session_store,
                submitted,
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
    resources: &LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    session_store: &SessionStore,
    tui: &mut TuiState,
    no_alt_screen: bool,
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
    state: &AppState,
    providers: &mut ProviderRegistry,
    auth_store: &AuthStore,
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
        "agents" if args.is_empty() => {
            let entries = load_agent_picker_entries(&state.cwd, state.current_model.as_deref())?;
            if entries.is_empty() {
                None
            } else {
                Some(OverlayState::AgentPicker {
                    entries,
                    selection: 0,
                })
            }
        }
        "login" if args.is_empty() => {
            let entries = providers
                .providers()
                .map(|provider| ModelPickerEntry {
                    selector: provider.id.clone(),
                    description: provider.display_name.clone(),
                })
                .collect::<Vec<_>>();
            if entries.is_empty() {
                None
            } else {
                Some(OverlayState::LoginPicker {
                    entries,
                    selection: 0,
                })
            }
        }
        "logout" if args.is_empty() => {
            let entries = auth_store
                .provider_ids()
                .map(|provider_id| ModelPickerEntry {
                    selector: provider_id.to_string(),
                    description: providers
                        .provider(provider_id)
                        .map(|provider| provider.display_name.clone())
                        .unwrap_or_else(|| provider_id.to_string()),
                })
                .collect::<Vec<_>>();
            if entries.is_empty() {
                None
            } else {
                Some(OverlayState::LogoutPicker {
                    entries,
                    selection: 0,
                })
            }
        }
        "theme" if args.is_empty() => Some(OverlayState::ThemePicker {
            entries: vec![
                ModelPickerEntry {
                    selector: "puffer".to_string(),
                    description: "Default Puffer theme".to_string(),
                },
                ModelPickerEntry {
                    selector: "harbor".to_string(),
                    description: "Harbor theme".to_string(),
                },
                ModelPickerEntry {
                    selector: "sunrise".to_string(),
                    description: "Warm light theme".to_string(),
                },
            ],
            selection: 0,
        }),
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

fn handle_auth_command(
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
    };
    let message = if auth_store.remove(provider).is_some() {
        auth_store.save(auth_path)?;
        format!("Removed stored credentials for {provider}.")
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

fn run_embedded_auth_login(
    provider: &str,
    auth_store: &mut AuthStore,
    auth_path: &Path,
    no_alt_screen: bool,
) -> Result<()> {
    if !no_alt_screen {
        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;
    }

    let status = Command::new(std::env::current_exe()?)
        .arg("auth")
        .arg("login")
        .arg(provider)
        .status()?;

    if !no_alt_screen {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
    }

    if !status.success() {
        anyhow::bail!("login flow for {provider} exited with {}", status);
    }

    *auth_store = AuthStore::load(auth_path)?;
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
    AgentPicker {
        entries: Vec<ModelPickerEntry>,
        selection: usize,
    },
    ModelPicker {
        entries: Vec<ModelPickerEntry>,
        selection: usize,
    },
    LoginPicker {
        entries: Vec<ModelPickerEntry>,
        selection: usize,
    },
    LogoutPicker {
        entries: Vec<ModelPickerEntry>,
        selection: usize,
    },
    ThemePicker {
        entries: Vec<ModelPickerEntry>,
        selection: usize,
    },
}

impl OverlayState {
    fn select_previous(&mut self) {
        match self {
            Self::SessionPicker { selection, .. }
            | Self::AgentPicker { selection, .. }
            | Self::ModelPicker { selection, .. }
            | Self::LoginPicker { selection, .. }
            | Self::LogoutPicker { selection, .. }
            | Self::ThemePicker { selection, .. } => {
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
            Self::AgentPicker { entries, selection } => {
                *selection = (*selection + 1).min(entries.len().saturating_sub(1));
            }
            Self::ModelPicker { entries, selection } => {
                *selection = (*selection + 1).min(entries.len().saturating_sub(1));
            }
            Self::LoginPicker { entries, selection } => {
                *selection = (*selection + 1).min(entries.len().saturating_sub(1));
            }
            Self::LogoutPicker { entries, selection }
            | Self::ThemePicker { entries, selection } => {
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
            Self::AgentPicker { entries, selection } => entries
                .get(*selection)
                .map(|entry| format!("/agents use {}", entry.selector)),
            Self::ModelPicker { entries, selection } => entries
                .get(*selection)
                .map(|entry| format!("/model {}", entry.selector)),
            Self::LoginPicker { entries, selection } => entries
                .get(*selection)
                .map(|entry| format!("/login {}", entry.selector)),
            Self::LogoutPicker { entries, selection } => entries
                .get(*selection)
                .map(|entry| format!("/logout {}", entry.selector)),
            Self::ThemePicker { entries, selection } => entries
                .get(*selection)
                .map(|entry| format!("/theme {}", entry.selector)),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ModelPickerEntry {
    pub selector: String,
    pub description: String,
}

fn load_agent_picker_entries(
    cwd: &Path,
    current_model: Option<&str>,
) -> Result<Vec<ModelPickerEntry>> {
    let paths = ConfigPaths::discover(cwd);
    ensure_workspace_dirs(&paths)?;
    let agents_path = paths.workspace_config_dir.join("agents.yaml");
    if !agents_path.exists() {
        fs::write(&agents_path, default_agents_contents(current_model))?;
    }
    let raw = fs::read_to_string(&agents_path)?;
    let parsed: AgentFile = serde_yaml::from_str(&raw)?;
    Ok(parsed
        .agents
        .into_iter()
        .map(|agent| ModelPickerEntry {
            selector: agent.id,
            description: format!("role={} model={}", agent.role, agent.model),
        })
        .collect())
}

fn default_agents_contents(model: Option<&str>) -> String {
    format!(
        "agents:\n  - id: default\n    role: coding\n    model: {}\n",
        model.unwrap_or("anthropic/claude-sonnet-4-5")
    )
}

#[derive(Debug, Clone, Deserialize)]
struct AgentFile {
    agents: Vec<AgentPickerRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct AgentPickerRecord {
    id: String,
    role: String,
    model: String,
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
