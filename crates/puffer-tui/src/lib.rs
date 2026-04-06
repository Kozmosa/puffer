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
};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;
use puffer_session_store::{SessionStore, TranscriptEvent};
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
                tui.slash_selection,
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
        KeyCode::Up => tui.select_previous(commands),
        KeyCode::Down => tui.select_next(commands),
        KeyCode::Backspace => tui.backspace(commands),
        KeyCode::Delete => tui.delete(commands),
        KeyCode::Tab => {
            let _ = tui.apply_selected_command(commands);
        }
        KeyCode::Enter => {
            if tui.complete_on_enter(commands) {
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
mod tests {
    use super::*;
    use puffer_config::PufferConfig;
    use puffer_provider_registry::{
        AuthMode, ModelDescriptor, OAuthCredential, ProviderDescriptor,
    };
    use puffer_resources::{
        IdeSpec, LoadedItem, MascotSpec, McpServerSpec, PluginCommandSpec, PluginSpec,
        PromptTemplate, SkillSpec, SourceInfo, SourceKind, ToolSpec,
    };
    use puffer_session_store::SessionMetadata;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use std::path::PathBuf;
    use uuid::Uuid;

    #[test]
    fn render_shows_command_popup_for_slash_input() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = sample_state();
        let resources = sample_resources();
        let providers = sample_providers();
        let auth_store = sample_auth_store();
        terminal
            .draw(|frame| {
                render::render(
                    frame,
                    &state,
                    &resources,
                    &providers,
                    &auth_store,
                    "/rev",
                    0,
                    &supported_commands(),
                )
            })
            .unwrap();
        let rendered = buffer_to_string(terminal.backend().buffer());
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("Inspector"));
        assert!(rendered.contains("/review"));
        assert!(rendered.contains("prompt"));
    }

    #[test]
    fn parse_shell_shortcut_accepts_bang_prefix() {
        assert_eq!(parse_shell_shortcut("!printf hi"), Some("printf hi"));
        assert_eq!(parse_shell_shortcut("!!printf hi"), Some("printf hi"));
        assert_eq!(parse_shell_shortcut("/help"), None);
    }

    #[test]
    fn slash_completion_appends_argument_space_for_commands_with_args() {
        let commands = supported_commands();
        let mut tui = TuiState::default();
        tui.insert_char('/', &commands);
        tui.insert_char('m', &commands);
        tui.insert_char('o', &commands);
        assert!(tui.apply_selected_command(&commands));
        assert_eq!(tui.input, "/model ");
        assert_eq!(tui.cursor, tui.input.len());
    }

    #[test]
    fn enter_completion_prefers_selected_slash_command() {
        let commands = supported_commands();
        let mut tui = TuiState::default();
        tui.insert_char('/', &commands);
        tui.insert_char('r', &commands);
        tui.insert_char('e', &commands);
        tui.insert_char('v', &commands);
        assert!(tui.complete_on_enter(&commands));
        assert_eq!(tui.input, "/review");
    }

    #[test]
    fn render_shows_status_line_when_enabled() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = sample_state();
        let resources = sample_resources();
        let providers = sample_providers();
        let auth_store = sample_auth_store();
        terminal
            .draw(|frame| {
                render::render(
                    frame,
                    &state,
                    &resources,
                    &providers,
                    &auth_store,
                    "",
                    0,
                    &supported_commands(),
                )
            })
            .unwrap();
        let rendered = buffer_to_string(terminal.backend().buffer());
        assert!(rendered.contains("Status Line"));
        assert!(rendered.contains("provider=anthropic"));
        assert!(rendered.contains("tools=3/3"));
        assert!(rendered.contains("Inspector"));
    }

    fn sample_state() -> AppState {
        let mut state = AppState::new(
            PufferConfig::default(),
            PathBuf::from("/workspace/puffer"),
            SessionMetadata {
                id: Uuid::nil(),
                display_name: Some("demo".to_string()),
                cwd: PathBuf::from("/workspace/puffer"),
                created_at_ms: 0,
                updated_at_ms: 0,
                parent_session_id: None,
                slug: Some("demo-session".to_string()),
                tags: vec!["review".to_string()],
                note: Some("Focus on transport parity".to_string()),
            },
        );
        state.statusline_enabled = true;
        state.current_provider = Some("anthropic".to_string());
        state.current_model = Some("anthropic/claude-sonnet-4-5".to_string());
        state.prompt_color = "amber".to_string();
        state.effort_level = "high".to_string();
        state.fast_mode = true;
        state.sandbox_mode = "workspace-write".to_string();
        state.remote_name = Some("buildbox".to_string());
        state.remote_environment = Some("linux".to_string());
        state.push_message(MessageRole::User, "/review");
        state.push_message(
            MessageRole::Assistant,
            "# Review\n- Check command coverage\n- Tighten request parity",
        );
        state
    }

    fn sample_resources() -> LoadedResources {
        LoadedResources {
            tools: vec![
                loaded_item(
                    "tools/bash.yaml",
                    ToolSpec {
                        id: "bash".to_string(),
                        name: "bash".to_string(),
                        description: "Run shell commands".to_string(),
                        handler: "bash".to_string(),
                        approval_policy: Some("on-request".to_string()),
                        sandbox_policy: Some("workspace-write".to_string()),
                    },
                ),
                loaded_item(
                    "tools/read_file.yaml",
                    ToolSpec {
                        id: "read_file".to_string(),
                        name: "read_file".to_string(),
                        description: "Read a file".to_string(),
                        handler: "read_file".to_string(),
                        approval_policy: Some("never".to_string()),
                        sandbox_policy: Some("read-only".to_string()),
                    },
                ),
                loaded_item(
                    "tools/write_file.yaml",
                    ToolSpec {
                        id: "write_file".to_string(),
                        name: "write_file".to_string(),
                        description: "Write a file".to_string(),
                        handler: "write_file".to_string(),
                        approval_policy: Some("on-request".to_string()),
                        sandbox_policy: Some("workspace-write".to_string()),
                    },
                ),
            ],
            prompts: vec![loaded_item(
                "prompts/review.yaml",
                PromptTemplate {
                    id: "review".to_string(),
                    description: "Review pending changes".to_string(),
                    template: "Review $ARGUMENTS".to_string(),
                },
            )],
            skills: vec![loaded_item(
                "skills/reviewer.yaml",
                SkillSpec {
                    name: "reviewer".to_string(),
                    description: "Code review helper".to_string(),
                    content: "Review code carefully".to_string(),
                    disable_model_invocation: false,
                },
            )],
            mascots: vec![loaded_item(
                "mascots/clawd.yaml",
                MascotSpec {
                    id: "clawd".to_string(),
                    display_name: "Clawd".to_string(),
                    introduction: "A diligent pufferfish".to_string(),
                },
            )],
            plugins: vec![loaded_item(
                "plugins/git.yaml",
                PluginSpec {
                    id: "git".to_string(),
                    display_name: "Git".to_string(),
                    description: "Git helpers".to_string(),
                    commands: vec![PluginCommandSpec {
                        name: "review".to_string(),
                        description: "Review a diff".to_string(),
                    }],
                    skills: vec!["reviewer".to_string()],
                    mcp_servers: vec![McpServerSpec {
                        id: "git-mcp".to_string(),
                        display_name: "Git MCP".to_string(),
                        transport: "stdio".to_string(),
                        endpoint: String::new(),
                        target: "git".to_string(),
                        description: "Git bridge".to_string(),
                    }],
                },
            )],
            mcp_servers: vec![loaded_item(
                "mcp_servers/local.yaml",
                McpServerSpec {
                    id: "local".to_string(),
                    display_name: "Local MCP".to_string(),
                    transport: "stdio".to_string(),
                    endpoint: String::new(),
                    target: "local".to_string(),
                    description: "Local tool bridge".to_string(),
                },
            )],
            ides: vec![loaded_item(
                "ides/vscode.yaml",
                IdeSpec {
                    id: "vscode".to_string(),
                    display_name: "VS Code".to_string(),
                    description: "VS Code bridge".to_string(),
                },
            )],
            ..LoadedResources::default()
        }
    }

    fn sample_providers() -> ProviderRegistry {
        let mut providers = ProviderRegistry::default();
        providers.register(ProviderDescriptor {
            id: "anthropic".to_string(),
            display_name: "Anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            default_api: "anthropic-messages".to_string(),
            auth_modes: vec![AuthMode::ApiKey, AuthMode::OAuth],
            headers: Default::default(),
            models: vec![
                ModelDescriptor {
                    id: "claude-sonnet-4-5".to_string(),
                    display_name: "Claude Sonnet 4.5".to_string(),
                    provider: "anthropic".to_string(),
                    api: "anthropic-messages".to_string(),
                    context_window: 200_000,
                    max_output_tokens: 8_192,
                    supports_reasoning: true,
                },
                ModelDescriptor {
                    id: "claude-opus-4-1".to_string(),
                    display_name: "Claude Opus 4.1".to_string(),
                    provider: "anthropic".to_string(),
                    api: "anthropic-messages".to_string(),
                    context_window: 200_000,
                    max_output_tokens: 8_192,
                    supports_reasoning: true,
                },
            ],
        });
        providers.register(ProviderDescriptor {
            id: "openai".to_string(),
            display_name: "OpenAI".to_string(),
            base_url: "https://api.openai.com".to_string(),
            default_api: "responses".to_string(),
            auth_modes: vec![AuthMode::ApiKey, AuthMode::OAuth],
            headers: Default::default(),
            models: vec![ModelDescriptor {
                id: "gpt-5".to_string(),
                display_name: "GPT-5".to_string(),
                provider: "openai".to_string(),
                api: "responses".to_string(),
                context_window: 200_000,
                max_output_tokens: 8_192,
                supports_reasoning: true,
            }],
        });
        providers
    }

    fn sample_auth_store() -> AuthStore {
        let mut auth_store = AuthStore::default();
        auth_store.set_oauth(
            "anthropic",
            OAuthCredential {
                access_token: "access".to_string(),
                refresh_token: "refresh".to_string(),
                expires_at_ms: 100,
                account_id: Some("acct".to_string()),
                email: Some("operator@example.com".to_string()),
                scopes: vec!["org:create_api_key".to_string()],
            },
        );
        auth_store
    }

    fn loaded_item<T>(path: &str, value: T) -> LoadedItem<T> {
        LoadedItem {
            value,
            source_info: SourceInfo {
                path: PathBuf::from(path),
                kind: SourceKind::Builtin,
            },
        }
    }

    fn buffer_to_string(buffer: &Buffer) -> String {
        let area = buffer.area();
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
