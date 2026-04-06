use crate::markdown::render_markdown;
use crate::popup::popup_rows;
use puffer_core::{AppState, CommandKind, CommandSpec, MessageRole, RenderedMessage};
use puffer_provider_registry::{AuthStore, StoredCredential};
use puffer_resources::LoadedResources;
use puffer_tools::{ToolKind, ToolRegistry};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use std::path::Path;

/// Renders the current application frame.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    state: &AppState,
    resources: &LoadedResources,
    providers: &puffer_provider_registry::ProviderRegistry,
    auth_store: &AuthStore,
    input: &str,
    slash_selection: usize,
    commands: &[CommandSpec],
) {
    let tool_registry = ToolRegistry::from_resources(resources);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(10),
            Constraint::Length(3),
            Constraint::Length(5),
        ])
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(layout[1]);

    let sidebar = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Min(10),
        ])
        .split(body[1]);

    let header = Paragraph::new(Text::from(header_lines(
        state,
        resources,
        auth_store,
        &tool_registry,
    )))
    .wrap(Wrap { trim: false })
    .block(Block::default().title("Header").borders(Borders::ALL));
    frame.render_widget(header, layout[0]);

    let transcript_widget = Paragraph::new(transcript_text(state))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Transcript").borders(Borders::ALL));
    frame.render_widget(transcript_widget, body[0]);

    let session = Paragraph::new(session_lines(state).join("\n"))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Session").borders(Borders::ALL));
    frame.render_widget(session, sidebar[0]);

    let tools = Paragraph::new(tool_lines(state, resources, &tool_registry).join("\n"))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Tools").borders(Borders::ALL));
    frame.render_widget(tools, sidebar[1]);

    let inspector = Paragraph::new(
        inspector_lines(state, resources, providers, auth_store, input, commands).join("\n"),
    )
    .wrap(Wrap { trim: false })
    .block(Block::default().title("Inspector").borders(Borders::ALL));
    frame.render_widget(inspector, sidebar[2]);

    let input_text = if input.is_empty() {
        "<type /help, /review, or !pwd>"
    } else {
        input
    };
    let input_widget =
        Paragraph::new(input_text).block(Block::default().title("Composer").borders(Borders::ALL));
    frame.render_widget(input_widget, layout[2]);

    let footer_title = if state.statusline_enabled {
        "Status Line"
    } else {
        "Footer"
    };
    let footer = Paragraph::new(Text::from(footer_lines(
        state,
        resources,
        auth_store,
        &tool_registry,
        input,
        commands,
    )))
    .wrap(Wrap { trim: false })
    .block(Block::default().title(footer_title).borders(Borders::ALL));
    frame.render_widget(footer, layout[3]);

    if input.starts_with('/') {
        render_command_popup(frame, body[0], input, slash_selection, commands);
    }
}

fn transcript_text(state: &AppState) -> Text<'static> {
    if state.transcript.is_empty() {
        return Text::from("No transcript yet. Type a prompt or a slash command.");
    }

    Text::from(
        state
            .transcript
            .iter()
            .flat_map(render_transcript_message)
            .collect::<Vec<_>>(),
    )
}

fn render_transcript_message(message: &RenderedMessage) -> Vec<Line<'static>> {
    let prefix = match message.role {
        MessageRole::User => "You",
        MessageRole::Assistant => "Puffer",
        MessageRole::System => "System",
    };
    render_markdown(&message.text)
        .lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                let mut spans = vec![format!("{prefix}: ").into()];
                spans.extend(line.spans);
                Line::from(spans)
            } else {
                let mut spans = vec!["        ".into()];
                spans.extend(line.spans);
                Line::from(spans)
            }
        })
        .collect()
}

fn header_lines(
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
) -> Vec<Line<'static>> {
    let session_name = session_name(state);
    let remote = remote_label(state);
    let tool_status = tool_status(tool_registry);
    vec![
        Line::from(format!(
            "Puffer Code  session={}  cwd={}  msgs={}",
            truncate(&session_name, 24),
            truncate(&path_tail(&state.session.cwd), 24),
            state.transcript.len(),
        )),
        Line::from(format!(
            "provider={}  model={}  auth={}  remote={}",
            current_provider(state),
            truncate(current_model(state), 34),
            auth_status(state, auth_store),
            truncate(&remote, 24),
        )),
        Line::from(format!(
            "slug={}  tags={}  mascot={}",
            state.session.slug.as_deref().unwrap_or("-"),
            truncate(&format_tag_summary(&state.session.tags), 24),
            mascot_label(state),
        )),
        Line::from(format!(
            "theme={}  effort={}  tools={}/{}  prompts={}",
            state.config.theme,
            state.effort_level,
            tool_status.executable,
            resources.tools.len(),
            resources.prompts.len(),
        )),
    ]
}

fn session_lines(state: &AppState) -> Vec<String> {
    let parent = state
        .session
        .parent_session_id
        .map(|value| short_id(&value.to_string()))
        .unwrap_or_else(|| "root".to_string());
    let note = state.session.note.as_deref().unwrap_or("-");
    vec![
        format!("Name: {}", truncate(&session_name(state), 26)),
        format!("Id: {}", short_id(&state.session.id.to_string())),
        format!("Parent: {parent}"),
        format!("Dir: {}", truncate(&path_tail(&state.cwd), 26)),
        format!("Transcript: {} messages", state.transcript.len()),
        format!("Workdirs: {}", state.working_dirs.len()),
        format!(
            "Tags: {}",
            truncate(&format_tag_summary(&state.session.tags), 26)
        ),
        format!("Note: {}", truncate(note, 26)),
    ]
}

fn tool_lines(
    state: &AppState,
    resources: &LoadedResources,
    tool_registry: &ToolRegistry,
) -> Vec<String> {
    let tool_status = tool_status(tool_registry);
    let activity = shell_activity(&state.transcript);
    let approval_policies = resources
        .tools
        .iter()
        .filter(|tool| tool.value.approval_policy.is_some())
        .count();
    let sandbox_policies = resources
        .tools
        .iter()
        .filter(|tool| tool.value.sandbox_policy.is_some())
        .count();
    vec![
        format!("Configured: {}", resources.tools.len()),
        format!("Executable: {}", tool_status.executable),
        format!(
            "Kinds: bash={} read={} write={}",
            toggle_word(tool_status.has_bash),
            toggle_word(tool_status.has_read_file),
            toggle_word(tool_status.has_write_file),
        ),
        format!("Policies: approval={approval_policies} sandbox={sandbox_policies}"),
        format!("Shell runs: {}", activity.total_runs),
        format!(
            "Last shell: {}",
            activity
                .last_command
                .as_deref()
                .map(|command| truncate(command, 24))
                .unwrap_or_else(|| "-".to_string())
        ),
        format!("Current sandbox: {}", state.sandbox_mode),
        format!("Prompt mode: {}", state.prompt_color),
    ]
}

fn inspector_lines(
    state: &AppState,
    resources: &LoadedResources,
    providers: &puffer_provider_registry::ProviderRegistry,
    auth_store: &AuthStore,
    input: &str,
    commands: &[CommandSpec],
) -> Vec<String> {
    let mut lines = vec![
        format!("Provider: {}", current_provider(state)),
        format!("Model: {}", truncate(current_model(state), 28)),
        format!("Auth: {}", auth_status(state, auth_store)),
        format!(
            "Auth providers: {}",
            truncate(&auth_provider_summary(auth_store), 28)
        ),
        format!("Theme: {}", state.config.theme),
        format!("Effort: {}", state.effort_level),
        format!("Fast mode: {}", toggle_word(state.fast_mode)),
        format!("Vim mode: {}", toggle_word(state.vim_mode)),
        format!("Status line: {}", toggle_word(state.statusline_enabled)),
        format!("Providers loaded: {}", providers.providers().count()),
        format!("Prompts: {}", resources.prompts.len()),
        format!(
            "Resources: skills={} plugins={} mcp={} ides={}",
            resources.skills.len(),
            resources.plugins.len(),
            resources.mcp_servers.len(),
            resources.ides.len(),
        ),
    ];

    if input.starts_with('/') {
        let rows = popup_rows(input, commands);
        let best = rows
            .first()
            .map(|command| format!("/{}", command.name))
            .unwrap_or_else(|| "<none>".to_string());
        lines.push(format!("Slash filter: {}", truncate(input, 24)));
        lines.push(format!("Slash matches: {} best={best}", rows.len()));
    } else {
        lines.push("Hint: / opens slash commands".to_string());
        lines.push("Hint: ! runs the bash tool".to_string());
    }

    lines
}

fn footer_lines(
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
    input: &str,
    commands: &[CommandSpec],
) -> Vec<Line<'static>> {
    if !state.statusline_enabled {
        return vec![
            Line::from("Enter submits  Esc clears  Ctrl+C exits  / opens commands"),
            Line::from(format!(
                "session={}  provider={}  tools={}",
                short_id(&state.session.id.to_string()),
                current_provider(state),
                resources.tools.len(),
            )),
            Line::from(footer_hint(input, commands)),
        ];
    }

    let tool_status = tool_status(tool_registry);
    let activity = shell_activity(&state.transcript);
    vec![
        Line::from(format!(
            "auth={}  provider={}  model={}  transcript={}",
            auth_status(state, auth_store),
            current_provider(state),
            truncate(current_model(state), 28),
            state.transcript.len(),
        )),
        Line::from(format!(
            "tools={}/{} ready  shell-runs={}  fast={}  vim={}  sandbox={}",
            tool_status.executable,
            resources.tools.len(),
            activity.total_runs,
            toggle_word(state.fast_mode),
            toggle_word(state.vim_mode),
            truncate(&state.sandbox_mode, 20),
        )),
        Line::from(footer_hint(input, commands)),
    ]
}

fn current_provider(state: &AppState) -> &str {
    state.current_provider.as_deref().unwrap_or("<unset>")
}

fn current_model(state: &AppState) -> &str {
    state.current_model.as_deref().unwrap_or("<unset>")
}

fn auth_status(state: &AppState, auth_store: &AuthStore) -> &'static str {
    match state
        .current_provider
        .as_deref()
        .and_then(|id| auth_store.get(id))
    {
        Some(StoredCredential::ApiKey { .. }) => "api-key",
        Some(StoredCredential::OAuth(_)) => "oauth",
        None if state.current_provider.is_some() => "missing",
        None => "n/a",
    }
}

fn auth_provider_summary(auth_store: &AuthStore) -> String {
    let providers = auth_store.provider_ids().collect::<Vec<_>>();
    if providers.is_empty() {
        "none".to_string()
    } else {
        providers.join(",")
    }
}

fn session_name(state: &AppState) -> String {
    state
        .session
        .display_name
        .as_deref()
        .or(state.session.slug.as_deref())
        .unwrap_or("untitled")
        .to_string()
}

fn mascot_label(state: &AppState) -> &str {
    if state.config.mascot.enabled {
        &state.config.mascot.display_name
    } else {
        "disabled"
    }
}

fn remote_label(state: &AppState) -> String {
    match (
        state.remote_name.as_deref(),
        state.remote_environment.as_deref(),
    ) {
        (Some(name), Some(environment)) => format!("{name}@{environment}"),
        (Some(name), None) => name.to_string(),
        (None, Some(environment)) => environment.to_string(),
        (None, None) => "local".to_string(),
    }
}

fn path_tail(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

fn short_id(value: &str) -> String {
    value.chars().take(8).collect()
}

fn format_tag_summary(tags: &[String]) -> String {
    if tags.is_empty() {
        "-".to_string()
    } else {
        tags.join(",")
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }
    let prefix = value.chars().take(max_chars - 3).collect::<String>();
    format!("{prefix}...")
}

fn toggle_word(enabled: bool) -> &'static str {
    if enabled {
        "on"
    } else {
        "off"
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ToolStatus {
    executable: usize,
    has_bash: bool,
    has_read_file: bool,
    has_write_file: bool,
}

fn tool_status(tool_registry: &ToolRegistry) -> ToolStatus {
    let mut status = ToolStatus::default();
    for tool in tool_registry.tools() {
        status.executable += 1;
        match tool.kind {
            ToolKind::Bash => status.has_bash = true,
            ToolKind::ReadFile => status.has_read_file = true,
            ToolKind::WriteFile => status.has_write_file = true,
        }
    }
    status
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ShellActivity {
    total_runs: usize,
    last_command: Option<String>,
}

fn shell_activity(messages: &[RenderedMessage]) -> ShellActivity {
    let mut activity = ShellActivity::default();
    for message in messages {
        if message.role != MessageRole::User {
            continue;
        }
        let Some(command) = shell_command_from_message(&message.text) else {
            continue;
        };
        activity.total_runs += 1;
        activity.last_command = Some(command.to_string());
    }
    activity
}

fn shell_command_from_message(text: &str) -> Option<&str> {
    let command = text.strip_prefix("!!").or_else(|| text.strip_prefix('!'))?;
    let trimmed = command.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn footer_hint(input: &str, commands: &[CommandSpec]) -> String {
    if input.starts_with('/') {
        let rows = popup_rows(input, commands);
        let best = rows
            .first()
            .map(|command| format!("/{}", command.name))
            .unwrap_or_else(|| "<none>".to_string());
        return format!(
            "slash={} matches={} best={}  Enter submits  Esc clears",
            truncate(input, 18),
            rows.len(),
            best,
        );
    }

    "/review reviews changes  /usage shows totals  !git status runs via bash".to_string()
}

fn render_command_popup(
    frame: &mut Frame<'_>,
    transcript_area: Rect,
    input: &str,
    slash_selection: usize,
    commands: &[CommandSpec],
) {
    let matching = popup_rows(input, commands)
        .into_iter()
        .enumerate()
        .map(|(index, command)| {
            let alias_suffix = if command.aliases.is_empty() {
                String::new()
            } else {
                format!(" [{}]", command.aliases.join(", "))
            };
            let argument_hint = command
                .argument_hint
                .map(|value| format!(" {value}"))
                .unwrap_or_default();
            ListItem::new(format!(
                "/{:<16} {:<6} {}{}{}",
                command.name,
                command_kind_label(command.kind),
                command.description,
                argument_hint,
                alias_suffix,
            ))
            .style(if index == slash_selection {
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::REVERSED)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            })
        })
        .collect::<Vec<_>>();

    let popup_area = Rect {
        x: transcript_area.x + 2,
        y: transcript_area.y + 2,
        width: transcript_area.width.saturating_sub(4).min(88),
        height: matching.len() as u16 + 2,
    };
    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        List::new(matching).block(Block::default().title("Commands").borders(Borders::ALL)),
        popup_area,
    );
}

fn command_kind_label(kind: CommandKind) -> &'static str {
    match kind {
        CommandKind::Prompt => "prompt",
        CommandKind::Local => "local",
        CommandKind::Ui => "ui",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use puffer_config::PufferConfig;
    use puffer_provider_registry::{
        AuthMode, ModelDescriptor, OAuthCredential, ProviderDescriptor, ProviderRegistry,
    };
    use puffer_resources::{LoadedItem, SourceInfo, SourceKind, ToolSpec};
    use puffer_session_store::SessionMetadata;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;
    use uuid::Uuid;

    #[test]
    fn session_lines_include_lineage_tags_and_note() {
        let state = sample_state();
        let lines = session_lines(&state).join("\n");
        assert!(lines.contains("Name: Shipyard"));
        assert!(lines.contains("Parent: abcdefab"));
        assert!(lines.contains("Tags: review,tmux"));
        assert!(lines.contains("Note: finish tool parity"));
    }

    #[test]
    fn tool_lines_include_registry_status_and_shell_activity() {
        let state = sample_state();
        let resources = sample_resources();
        let registry = ToolRegistry::from_resources(&resources);
        let lines = tool_lines(&state, &resources, &registry).join("\n");
        assert!(lines.contains("Configured: 4"));
        assert!(lines.contains("Executable: 3"));
        assert!(lines.contains("Kinds: bash=on read=on write=on"));
        assert!(lines.contains("Shell runs: 1"));
        assert!(lines.contains("Last shell: git status"));
    }

    #[test]
    fn header_snapshot_reports_runtime_counts() {
        let state = sample_state();
        let resources = sample_resources();
        let auth_store = sample_auth_store();
        let registry = ToolRegistry::from_resources(&resources);
        let snapshot = header_lines(&state, &resources, &auth_store, &registry)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_snapshot!(
                                            snapshot,
                                            @r"
Puffer Code  session=Shipyard  cwd=puffer  msgs=2
provider=anthropic  model=anthropic/claude-sonnet-4-5  auth=api-key  remote=dockyard@staging
slug=shipyard-run  tags=review,tmux  mascot=Clawd
theme=harbor  effort=high  tools=3/4  prompts=2
"
                                        );
    }

    #[test]
    fn inspector_snapshot_reports_slash_state() {
        let state = sample_state();
        let resources = sample_resources();
        let providers = sample_providers();
        let auth_store = sample_auth_store();
        let snapshot = inspector_lines(
            &state,
            &resources,
            &providers,
            &auth_store,
            "/re",
            &sample_commands(),
        )
        .join("\n");
        assert_snapshot!(
                                            snapshot,
                                            @r"
Provider: anthropic
Model: anthropic/claude-sonnet-4-5
Auth: api-key
Auth providers: anthropic,openai
Theme: harbor
Effort: high
Fast mode: on
Vim mode: on
Status line: on
Providers loaded: 2
Prompts: 2
Resources: skills=2 plugins=1 mcp=1 ides=1
Slash filter: /re
Slash matches: 2 best=/review
"
                                        );
    }

    #[test]
    fn footer_snapshot_reports_slash_hint_and_tool_state() {
        let state = sample_state();
        let resources = sample_resources();
        let auth_store = sample_auth_store();
        let registry = ToolRegistry::from_resources(&resources);
        let snapshot = footer_lines(
            &state,
            &resources,
            &auth_store,
            &registry,
            "/re",
            &sample_commands(),
        )
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        assert_snapshot!(
                                            snapshot,
                                            @r"
auth=api-key  provider=anthropic  model=anthropic/claude-sonnet-4-5  transcript=2
tools=3/4 ready  shell-runs=1  fast=on  vim=on  sandbox=workspace-write
slash=/re matches=2 best=/review  Enter submits  Esc clears
"
                                        );
    }

    #[test]
    fn render_draws_session_tool_and_status_surfaces() {
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = sample_state();
        let resources = sample_resources();
        let providers = sample_providers();
        let auth_store = sample_auth_store();

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &state,
                    &resources,
                    &providers,
                    &auth_store,
                    "/re",
                    0,
                    &sample_commands(),
                )
            })
            .unwrap();

        let rendered = terminal_view(&terminal);
        assert!(rendered.contains("Session"));
        assert!(rendered.contains("Tools"));
        assert!(rendered.contains("Inspector"));
        assert!(rendered.contains("Status Line"));
        assert!(rendered.contains("slash=/re matches=2 best=/review"));
        assert!(rendered.contains("Commands"));
    }

    fn sample_state() -> AppState {
        let mut config = PufferConfig::default();
        config.theme = "harbor".to_string();
        config.default_provider = Some("anthropic".to_string());
        config.default_model = Some("anthropic/claude-sonnet-4-5".to_string());
        config.mascot.display_name = "Clawd".to_string();

        let cwd = PathBuf::from("/tmp/puffer");
        let mut state = AppState::new(
            config,
            cwd.clone(),
            SessionMetadata {
                id: Uuid::parse_str("12345678-1234-5678-1234-567812345678").unwrap(),
                display_name: Some("Shipyard".to_string()),
                cwd,
                created_at_ms: 0,
                updated_at_ms: 0,
                parent_session_id: Some(
                    Uuid::parse_str("abcdefab-1111-2222-3333-444455556666").unwrap(),
                ),
                slug: Some("shipyard-run".to_string()),
                tags: vec!["review".to_string(), "tmux".to_string()],
                note: Some("finish tool parity".to_string()),
            },
        );
        state.current_provider = Some("anthropic".to_string());
        state.current_model = Some("anthropic/claude-sonnet-4-5".to_string());
        state.prompt_color = "cyan".to_string();
        state.effort_level = "high".to_string();
        state.fast_mode = true;
        state.vim_mode = true;
        state.remote_name = Some("dockyard".to_string());
        state.remote_environment = Some("staging".to_string());
        state.working_dirs = vec![
            PathBuf::from("/tmp/puffer"),
            PathBuf::from("/tmp/puffer/ref"),
        ];
        state.push_message(MessageRole::User, "!git status");
        state.push_message(MessageRole::Assistant, "working tree clean");
        state
    }

    fn sample_resources() -> LoadedResources {
        LoadedResources {
            tools: vec![
                tool_spec("bash", "bash", Some("on-request"), Some("workspace-write")),
                tool_spec("read_file", "read_file", None, Some("workspace-write")),
                tool_spec(
                    "write_file",
                    "write_file",
                    Some("on-request"),
                    Some("workspace-write"),
                ),
                tool_spec("custom_fetch", "custom_fetch", None, None),
            ],
            prompts: vec![
                loaded_prompt("review", "Review current work"),
                loaded_prompt("usage", "Show usage"),
            ],
            skills: vec![
                loaded_skill("rust-review", "Rust review guidance"),
                loaded_skill("tmux-parity", "tmux parity guidance"),
            ],
            plugins: vec![loaded_plugin("git", "Git plugin")],
            mcp_servers: vec![loaded_mcp("docs", "Docs server")],
            ides: vec![loaded_ide("zed", "Zed integration")],
            ..LoadedResources::default()
        }
    }

    fn sample_providers() -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderDescriptor {
            id: "anthropic".to_string(),
            display_name: "Anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            default_api: "anthropic-messages".to_string(),
            auth_modes: vec![AuthMode::ApiKey, AuthMode::OAuth],
            headers: Default::default(),
            models: vec![ModelDescriptor {
                id: "claude-sonnet-4-5".to_string(),
                display_name: "Claude Sonnet 4.5".to_string(),
                provider: "anthropic".to_string(),
                api: "anthropic-messages".to_string(),
                context_window: 200_000,
                max_output_tokens: 8_192,
                supports_reasoning: true,
            }],
        });
        registry.register(ProviderDescriptor {
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
                context_window: 128_000,
                max_output_tokens: 8_192,
                supports_reasoning: true,
            }],
        });
        registry
    }

    fn sample_auth_store() -> AuthStore {
        let mut store = AuthStore::default();
        store.set_api_key("anthropic", "sk-ant");
        store.set_oauth(
            "openai",
            OAuthCredential {
                access_token: "access".to_string(),
                refresh_token: "refresh".to_string(),
                expires_at_ms: 42,
                account_id: Some("acct-1".to_string()),
                email: Some("dev@example.com".to_string()),
                scopes: vec!["openid".to_string()],
            },
        );
        store
    }

    fn sample_commands() -> Vec<CommandSpec> {
        vec![
            CommandSpec {
                name: "review",
                aliases: &[],
                description: "Review current changes",
                argument_hint: None,
                kind: CommandKind::Prompt,
            },
            CommandSpec {
                name: "rewind",
                aliases: &["checkpoint"],
                description: "Restore to a prior point",
                argument_hint: None,
                kind: CommandKind::Ui,
            },
            CommandSpec {
                name: "usage",
                aliases: &[],
                description: "Show usage limits",
                argument_hint: None,
                kind: CommandKind::Local,
            },
        ]
    }

    fn tool_spec(
        id: &str,
        handler: &str,
        approval_policy: Option<&str>,
        sandbox_policy: Option<&str>,
    ) -> LoadedItem<ToolSpec> {
        LoadedItem {
            value: ToolSpec {
                id: id.to_string(),
                name: id.to_string(),
                description: format!("{id} tool"),
                handler: handler.to_string(),
                approval_policy: approval_policy.map(str::to_string),
                sandbox_policy: sandbox_policy.map(str::to_string),
            },
            source_info: SourceInfo {
                path: PathBuf::from(format!("{id}.yaml")),
                kind: SourceKind::Builtin,
            },
        }
    }

    fn loaded_prompt(id: &str, description: &str) -> LoadedItem<puffer_resources::PromptTemplate> {
        LoadedItem {
            value: puffer_resources::PromptTemplate {
                id: id.to_string(),
                description: description.to_string(),
                template: "template".to_string(),
            },
            source_info: SourceInfo {
                path: PathBuf::from(format!("{id}.yaml")),
                kind: SourceKind::Builtin,
            },
        }
    }

    fn loaded_skill(id: &str, description: &str) -> LoadedItem<puffer_resources::SkillSpec> {
        LoadedItem {
            value: puffer_resources::SkillSpec {
                name: id.to_string(),
                description: description.to_string(),
                content: "content".to_string(),
                disable_model_invocation: false,
            },
            source_info: SourceInfo {
                path: PathBuf::from(format!("{id}.yaml")),
                kind: SourceKind::Builtin,
            },
        }
    }

    fn loaded_plugin(id: &str, display_name: &str) -> LoadedItem<puffer_resources::PluginSpec> {
        LoadedItem {
            value: puffer_resources::PluginSpec {
                id: id.to_string(),
                display_name: display_name.to_string(),
                description: "plugin".to_string(),
                commands: Vec::new(),
                skills: Vec::new(),
                mcp_servers: Vec::new(),
            },
            source_info: SourceInfo {
                path: PathBuf::from(format!("{id}.yaml")),
                kind: SourceKind::Builtin,
            },
        }
    }

    fn loaded_mcp(id: &str, display_name: &str) -> LoadedItem<puffer_resources::McpServerSpec> {
        LoadedItem {
            value: puffer_resources::McpServerSpec {
                id: id.to_string(),
                display_name: display_name.to_string(),
                transport: "stdio".to_string(),
                endpoint: String::new(),
                target: "cargo run".to_string(),
                description: "mcp".to_string(),
            },
            source_info: SourceInfo {
                path: PathBuf::from(format!("{id}.yaml")),
                kind: SourceKind::Builtin,
            },
        }
    }

    fn loaded_ide(id: &str, display_name: &str) -> LoadedItem<puffer_resources::IdeSpec> {
        LoadedItem {
            value: puffer_resources::IdeSpec {
                id: id.to_string(),
                display_name: display_name.to_string(),
                description: "ide".to_string(),
            },
            source_info: SourceInfo {
                path: PathBuf::from(format!("{id}.yaml")),
                kind: SourceKind::Builtin,
            },
        }
    }

    fn terminal_view(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        buffer
            .content
            .chunks(width)
            .map(|row| {
                row.iter()
                    .map(|cell| cell.symbol())
                    .collect::<Vec<_>>()
                    .join("")
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
