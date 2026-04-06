use crate::markdown::render_markdown;
use crate::popup::popup_rows;
use crate::{ModelPickerEntry, OverlayState};
use puffer_core::{AppState, CommandKind, CommandSpec, MessageRole, RenderedMessage};
use puffer_provider_registry::{AuthStore, StoredCredential};
use puffer_resources::LoadedResources;
use puffer_tools::{ToolKind, ToolRegistry};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use std::cell::RefCell;
use std::path::Path;

thread_local! {
    static ACTIVE_OVERLAY: RefCell<Option<OverlayState>> = const { RefCell::new(None) };
}

/// Sets the active overlay rendered by the TUI on the next draw.
pub(crate) fn set_active_overlay(overlay: Option<OverlayState>) {
    ACTIVE_OVERLAY.with(|value| {
        *value.borrow_mut() = overlay;
    });
}

/// Renders the current application frame.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    state: &AppState,
    resources: &LoadedResources,
    providers: &puffer_provider_registry::ProviderRegistry,
    auth_store: &AuthStore,
    input: &str,
    cursor: usize,
    slash_selection: usize,
    scroll_offset: u16,
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
        .scroll((scroll_offset, 0))
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
    let max_cursor = usize::from(layout[2].width.saturating_sub(3));
    frame.set_cursor_position((
        layout[2].x + 1 + cursor.min(max_cursor) as u16,
        layout[2].y + 1,
    ));

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
    ACTIVE_OVERLAY.with(|value| {
        if let Some(overlay) = value.borrow().as_ref() {
            render_overlay(frame, body[0], overlay);
        }
    });
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

pub(crate) fn transcript_line_count(state: &AppState) -> u16 {
    transcript_text(state).lines.len().min(u16::MAX as usize) as u16
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
            "Kinds: bash={} read={} write={} replace={} list={} search={}",
            toggle_word(tool_status.has_bash),
            toggle_word(tool_status.has_read_file),
            toggle_word(tool_status.has_write_file),
            toggle_word(tool_status.has_replace_in_file),
            toggle_word(tool_status.has_list_dir),
            toggle_word(tool_status.has_search_text),
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
    has_replace_in_file: bool,
    has_list_dir: bool,
    has_search_text: bool,
}

fn tool_status(tool_registry: &ToolRegistry) -> ToolStatus {
    let mut status = ToolStatus::default();
    for tool in tool_registry.tools() {
        status.executable += 1;
        match tool.kind {
            ToolKind::Bash => status.has_bash = true,
            ToolKind::ReadFile => status.has_read_file = true,
            ToolKind::WriteFile => status.has_write_file = true,
            ToolKind::ReplaceInFile => status.has_replace_in_file = true,
            ToolKind::ListDir => status.has_list_dir = true,
            ToolKind::SearchText => status.has_search_text = true,
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

fn render_overlay(frame: &mut Frame<'_>, transcript_area: Rect, overlay: &OverlayState) {
    let area = Rect {
        x: transcript_area.x + 2,
        y: transcript_area.y + 1,
        width: transcript_area.width.saturating_sub(4).min(88),
        height: overlay_rows(overlay).len() as u16 + 2,
    };
    let rows = overlay_rows(overlay)
        .into_iter()
        .map(|row| {
            ListItem::new(row.text).style(if row.selected {
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            })
        })
        .collect::<Vec<_>>();
    frame.render_widget(Clear, area);
    frame.render_widget(
        List::new(rows).block(Block::default().title(overlay_title(overlay)).borders(Borders::ALL)),
        area,
    );
}

fn overlay_title(overlay: &OverlayState) -> &'static str {
    match overlay {
        OverlayState::SessionPicker { .. } => "Resume Session",
        OverlayState::ModelPicker { .. } => "Select Model",
        OverlayState::LoginPicker { .. } => "Login Provider",
        OverlayState::LogoutPicker { .. } => "Logout Provider",
        OverlayState::ThemePicker { .. } => "Select Theme",
    }
}

fn overlay_rows(overlay: &OverlayState) -> Vec<OverlayRow> {
    match overlay {
        OverlayState::SessionPicker {
            sessions,
            selection,
        } => sessions
            .iter()
            .enumerate()
            .map(|(index, session)| OverlayRow {
                selected: index == *selection,
                text: format!(
                    "{}  {}",
                    short_id(&session.id.to_string()),
                    session.display_name.as_deref().unwrap_or("<unnamed>")
                ),
            })
            .collect(),
        OverlayState::ModelPicker { entries, selection } => entries
            .iter()
            .enumerate()
            .map(|(index, entry)| OverlayRow {
                selected: index == *selection,
                text: render_model_entry(entry),
            })
            .collect(),
        OverlayState::LoginPicker { entries, selection } => entries
            .iter()
            .enumerate()
            .map(|(index, entry)| OverlayRow {
                selected: index == *selection,
                text: render_model_entry(entry),
            })
            .collect(),
        OverlayState::LogoutPicker { entries, selection }
        | OverlayState::ThemePicker { entries, selection } => entries
            .iter()
            .enumerate()
            .map(|(index, entry)| OverlayRow {
                selected: index == *selection,
                text: render_model_entry(entry),
            })
            .collect(),
    }
}

fn render_model_entry(entry: &ModelPickerEntry) -> String {
    format!("{}  {}", entry.selector, entry.description)
}

struct OverlayRow {
    text: String,
    selected: bool,
}

#[cfg(test)]
mod tests;
