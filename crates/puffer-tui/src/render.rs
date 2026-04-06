mod summary;

use self::summary::{
    hint_line, status_primary_line, status_secondary_line, top_panel_columns, top_panel_height,
};
#[cfg(test)]
use self::summary::{footer_lines, header_lines, session_lines};
use crate::markdown::render_markdown;
use crate::popup::popup_rows;
use crate::state::AuthPickerEntry;
use crate::{ModelPickerEntry, OverlayState};
use puffer_core::{AppState, CommandSpec, MessageRole, RenderedMessage};
use puffer_provider_registry::AuthStore;
use puffer_resources::LoadedResources;
use puffer_tools::ToolRegistry;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::symbols::border;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use std::cell::RefCell;

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
    _providers: &puffer_provider_registry::ProviderRegistry,
    auth_store: &AuthStore,
    input: &str,
    cursor: usize,
    slash_selection: usize,
    scroll_offset: u16,
    commands: &[CommandSpec],
) {
    let tool_registry = ToolRegistry::from_resources(resources);
    let active_overlay = ACTIVE_OVERLAY.with(|value| value.borrow().clone());
    let onboarding_active = active_overlay
        .as_ref()
        .map(OverlayState::is_onboarding)
        .unwrap_or(false);
    let footer_height = if onboarding_active {
        4
    } else if state.statusline_enabled {
        6
    } else {
        5
    };
    let header_height = top_panel_height(state, resources, auth_store, &tool_registry)
        .min(frame.area().height.saturating_sub(footer_height + 1));
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(8),
            Constraint::Length(footer_height),
        ])
        .split(frame.area());

    if header_height > 0 {
        render_top_panel(frame, layout[0], state, resources, auth_store, &tool_registry);
    }

    if let Some(help_text) = active_help_text(state, &active_overlay) {
        render_help_pane(frame, layout[1], state, help_text);
    } else if state.transcript.is_empty() && !onboarding_active {
        render_empty_state(frame, layout[1], state);
    } else {
        frame.render_widget(
            Paragraph::new(transcript_text(state))
                .scroll((scroll_offset, 0))
                .wrap(Wrap { trim: false }),
            layout[1],
        );
    }

    let footer_block = Block::default()
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(prompt_border_style(state));
    frame.render_widget(&footer_block, layout[2]);
    let footer_area = footer_block.inner(layout[2]);
    let footer = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if onboarding_active {
            [
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(0),
                Constraint::Length(0),
            ]
            .as_ref()
        } else if state.statusline_enabled {
            [
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ]
            .as_ref()
        } else {
            [
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(0),
            ]
            .as_ref()
        })
        .split(footer_area);

    if state.statusline_enabled && !onboarding_active {
        frame.render_widget(
            Paragraph::new(status_primary_line(
                state,
                resources,
                auth_store,
                &tool_registry,
            ))
            .style(Style::default().add_modifier(Modifier::DIM)),
            footer[0],
        );
        frame.render_widget(
            Paragraph::new(status_secondary_line(state, resources, &tool_registry))
                .style(Style::default().add_modifier(Modifier::DIM)),
            footer[1],
        );
    }

    let prompt_row = if state.statusline_enabled && !onboarding_active {
        footer[2]
    } else {
        footer[0]
    };
    let hint_row = if state.statusline_enabled && !onboarding_active {
        footer[3]
    } else {
        footer[1]
    };
    let summary_row = if state.statusline_enabled && !onboarding_active {
        None
    } else {
        Some(footer[2])
    };

    let overlay_active = active_overlay.is_some();
    if overlay_active {
        frame.render_widget(Paragraph::new(overlay_prompt_line(input)), prompt_row);
        let max_cursor = usize::from(prompt_row.width.saturating_sub(3));
        frame.set_cursor_position((
            prompt_row.x + 2 + cursor.min(max_cursor) as u16,
            prompt_row.y,
        ));
        frame.render_widget(
            Paragraph::new(overlay_hint_line(input, onboarding_active))
                .style(Style::default().add_modifier(Modifier::DIM)),
            hint_row,
        );
    } else {
        frame.render_widget(Paragraph::new(prompt_line(input)), prompt_row);
        let max_cursor = usize::from(prompt_row.width.saturating_sub(3));
        frame.set_cursor_position((
            prompt_row.x + 2 + cursor.min(max_cursor) as u16,
            prompt_row.y,
        ));

        frame.render_widget(
            Paragraph::new(hint_line(input, commands))
                .style(Style::default().add_modifier(Modifier::DIM)),
            hint_row,
        );
        if let Some(summary_row) = summary_row {
            frame.render_widget(
                Paragraph::new(status_primary_line(
                    state,
                    resources,
                    auth_store,
                    &tool_registry,
                ))
                .style(Style::default().add_modifier(Modifier::DIM)),
                summary_row,
            );
        }
    }

    if input.starts_with('/') && !onboarding_active {
        render_command_popup(frame, layout[1], prompt_row, input, slash_selection, commands);
    }
    if let Some(overlay) = active_overlay.as_ref() {
        render_overlay(frame, layout[1], overlay);
    }
}

fn render_top_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
) {
    let [left, right] = top_panel_columns(state, resources, auth_store, tool_registry);
    let block = Block::default()
        .title(" Puffer Code ")
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(prompt_border_style(state));
    frame.render_widget(&block, area);
    let inner = block.inner(area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(44),
            Constraint::Length(1),
            Constraint::Percentage(56),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(Text::from(left)).wrap(Wrap { trim: false }),
        columns[0],
    );
    if columns[1].width > 0 {
        let separator = (0..columns[1].height)
            .map(|_| {
                Line::from(Span::styled(
                    "│",
                    Style::default().add_modifier(Modifier::DIM),
                ))
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(Text::from(separator)), columns[1]);
    }
    frame.render_widget(
        Paragraph::new(Text::from(right)).wrap(Wrap { trim: false }),
        columns[2],
    );
}

fn active_help_text<'a>(
    state: &'a AppState,
    active_overlay: &Option<OverlayState>,
) -> Option<&'a str> {
    if !state.config.ui.tmux_golden_mode || active_overlay.is_some() {
        return None;
    }
    state.transcript.last().and_then(|message| {
        (message.role == MessageRole::System && message.text.starts_with("Supported commands:"))
            .then_some(message.text.as_str())
    })
}

fn transcript_text(state: &AppState) -> Text<'static> {
    if state.transcript.is_empty() {
        return Text::default();
    }

    let mut lines = Vec::new();
    for (index, message) in state.transcript.iter().enumerate() {
        if index > 0 {
            lines.push(Line::default());
        }
        lines.extend(render_transcript_message(message));
    }
    Text::from(lines)
}

pub(crate) fn transcript_line_count(state: &AppState) -> u16 {
    transcript_text(state).lines.len().min(u16::MAX as usize) as u16
}

fn render_transcript_message(message: &RenderedMessage) -> Vec<Line<'static>> {
    let (first_prefix, continuation_prefix) = match message.role {
        MessageRole::User => ("› ", "  "),
        MessageRole::Assistant => ("", "  "),
        MessageRole::System => ("· ", "  "),
    };
    render_markdown(&message.text)
        .lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 && !first_prefix.is_empty() {
                let mut spans = vec![Span::raw(first_prefix.to_string())];
                spans.extend(line.spans);
                Line::from(spans)
            } else if index > 0 && !continuation_prefix.is_empty() {
                let mut spans = vec![Span::raw(continuation_prefix.to_string())];
                spans.extend(line.spans);
                Line::from(spans)
            } else {
                line
            }
        })
        .collect()
}

fn render_empty_state(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let model = if state.current_model.is_none() {
        "/model to choose a model".to_string()
    } else {
        state.current_model.clone().unwrap_or_default()
    };
    let mascot = if state.config.mascot.enabled {
        format!("{} on duty", state.config.mascot.display_name)
    } else {
        "Mascot disabled".to_string()
    };
    let text = Text::from(vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            mascot,
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(Span::styled(model, Style::default().add_modifier(Modifier::DIM))),
        Line::from(Span::styled(
            state.cwd.display().to_string(),
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Review changes, ask a question, or type /",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "? for shortcuts · /help to begin",
            Style::default().add_modifier(Modifier::DIM),
        )),
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_help_pane(frame: &mut Frame<'_>, area: Rect, state: &AppState, text: &str) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(prompt_border_style(state));
    frame.render_widget(&block, area);
    let inner = block.inner(area);
    let content = Rect {
        x: inner.x.saturating_add(2),
        y: inner.y,
        width: inner.width.saturating_sub(4),
        height: inner.height,
    };
    frame.render_widget(
        Paragraph::new(text.to_string()).wrap(Wrap { trim: false }),
        content,
    );
}

fn prompt_line(input: &str) -> Line<'static> {
    if input.is_empty() {
        Line::from(vec![
            Span::raw("❯ "),
            Span::styled(
                "Review changes, ask a question, or type /",
                Style::default().add_modifier(Modifier::DIM),
            ),
        ])
    } else {
        Line::from(format!("❯ {input}"))
    }
}

fn overlay_prompt_line(input: &str) -> Line<'static> {
    if input.is_empty() {
        Line::from(vec![
            Span::raw("❯ "),
            Span::styled(
                "Type to jump",
                Style::default().add_modifier(Modifier::DIM),
            ),
        ])
    } else {
        Line::from(format!("❯ {input}"))
    }
}

fn overlay_hint_line(input: &str, onboarding_active: bool) -> String {
    let prefix = if input.is_empty() {
        "Type to jump"
    } else {
        "Typing jumps selection"
    };
    if onboarding_active {
        format!("{prefix} · Enter to continue · Esc to go back")
    } else {
        format!("{prefix} · Enter to select · Esc to close")
    }
}

fn short_id(value: &str) -> String {
    value.chars().take(8).collect()
}

fn render_command_popup(
    frame: &mut Frame<'_>,
    transcript_area: Rect,
    prompt_row: Rect,
    input: &str,
    slash_selection: usize,
    commands: &[CommandSpec],
) {
    let matching = popup_rows(input, commands)
        .into_iter()
        .enumerate()
        .map(|(index, command)| {
            let argument_hint = command
                .argument_hint
                .map(|value| format!("  {value}"))
                .unwrap_or_default();
            ListItem::new(format!(
                "/{:<16} {}{}",
                command.name, command.description, argument_hint
            ))
            .style(if index == slash_selection {
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            })
        })
        .collect::<Vec<_>>();

    let height = matching.len() as u16 + 2;
    let popup_area = Rect {
        x: transcript_area.x + 2,
        y: prompt_row
            .y
            .saturating_sub(height.saturating_add(1))
            .max(transcript_area.y + 1),
        width: transcript_area.width.saturating_sub(4).min(72),
        height,
    };
    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        List::new(matching).block(
            Block::default()
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(accent_border_style()),
        ),
        popup_area,
    );
}

fn render_overlay(frame: &mut Frame<'_>, viewport: Rect, overlay: &OverlayState) {
    if is_onboarding_overlay(overlay) {
        render_onboarding_overlay(frame, viewport, overlay);
        return;
    }
    if matches!(overlay, OverlayState::LoginPicker { .. }) {
        render_login_overlay(frame, viewport, overlay);
        return;
    }
    let width = viewport.width.saturating_sub(8).min(72);
    let height = overlay_rows(overlay).len() as u16 + 2;
    let area = Rect {
        x: viewport.x + viewport.width.saturating_sub(width) / 2,
        y: viewport.y + viewport.height.saturating_sub(height) / 2,
        width,
        height,
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
        List::new(rows).block(
            Block::default()
                .title(overlay_title(overlay))
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(accent_border_style()),
        ),
        area,
    );
}

fn render_login_overlay(frame: &mut Frame<'_>, viewport: Rect, overlay: &OverlayState) {
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
    let width = viewport.width.saturating_sub(8).min(72);
    let height = rows.len() as u16 + 4;
    let area = Rect {
        x: viewport.x + viewport.width.saturating_sub(width) / 2,
        y: viewport.y + viewport.height.saturating_sub(height) / 2,
        width,
        height,
    };
    let block = Block::default()
        .title(" Login ")
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(accent_border_style());
    frame.render_widget(Clear, area);
    frame.render_widget(&block, area);
    let inner = block.inner(area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    frame.render_widget(
        Paragraph::new("Select Provider").style(Style::default().add_modifier(Modifier::DIM)),
        sections[0],
    );
    frame.render_widget(List::new(rows), sections[1]);
}

fn overlay_title(overlay: &OverlayState) -> &'static str {
    match overlay {
        OverlayState::SessionPicker { .. } => "Resume Session",
        OverlayState::AgentPicker { .. } => "Select Agent",
        OverlayState::ModelPicker { .. } => "Select Model",
        OverlayState::ProviderPicker { .. } => "Select Provider",
        OverlayState::AuthPicker { .. } => "Select Login Method",
        OverlayState::ApiKeyPrompt { .. } => "Enter API Key",
        OverlayState::LoginPicker { .. } => "Select Provider",
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
        OverlayState::AgentPicker { entries, selection }
        | OverlayState::ModelPicker {
            entries, selection, ..
        }
        | OverlayState::ProviderPicker {
            entries, selection, ..
        }
        | OverlayState::LoginPicker { entries, selection }
        | OverlayState::LogoutPicker { entries, selection }
        | OverlayState::ThemePicker { entries, selection } => entries
            .iter()
            .enumerate()
            .map(|(index, entry)| OverlayRow {
                selected: index == *selection,
                text: render_model_entry(entry),
            })
            .collect(),
        OverlayState::AuthPicker { entries, selection, .. } => entries
            .iter()
            .enumerate()
            .map(|(index, entry)| OverlayRow {
                selected: index == *selection,
                text: render_auth_entry(entry),
            })
            .collect(),
        OverlayState::ApiKeyPrompt { value, .. } => vec![
            OverlayRow {
                selected: false,
                text: "Paste an API key and press Enter.".to_string(),
            },
            OverlayRow {
                selected: true,
                text: format!("key  {}", masked_secret(value)),
            },
        ],
    }
}

fn render_model_entry(entry: &ModelPickerEntry) -> String {
    format!("{}  {}", entry.selector, entry.description)
}

fn render_auth_entry(entry: &AuthPickerEntry) -> String {
    format!("{}  {}", entry.label, entry.description)
}

fn masked_secret(value: &str) -> String {
    if value.is_empty() {
        return "<empty>".to_string();
    }
    "*".repeat(value.chars().count().min(32))
}

fn accent_border_style() -> Style {
    Style::default().fg(Color::Cyan)
}

fn prompt_border_style(state: &AppState) -> Style {
    let color = match state.prompt_color.as_str() {
        "amber" | "yellow" => Color::Yellow,
        "teal" | "cyan" => Color::Cyan,
        "green" => Color::Green,
        "blue" => Color::Blue,
        "red" => Color::Red,
        "magenta" | "pink" => Color::Magenta,
        _ => Color::Cyan,
    };
    Style::default().fg(color)
}

struct OverlayRow {
    text: String,
    selected: bool,
}

fn is_onboarding_overlay(overlay: &OverlayState) -> bool {
    overlay.is_onboarding()
}

fn render_onboarding_overlay(frame: &mut Frame<'_>, viewport: Rect, overlay: &OverlayState) {
    let body_lines = onboarding_body_lines(overlay);
    let width = viewport.width.saturating_sub(12).min(76).max(34);
    let height = (body_lines.len() as u16 + 2).min(viewport.height.saturating_sub(2));
    let area = Rect {
        x: viewport.x + viewport.width.saturating_sub(width) / 2,
        y: viewport.y + viewport.height.saturating_sub(height) / 3,
        width,
        height,
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Text::from(body_lines))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_set(border::ROUNDED)
                    .border_style(accent_border_style()),
            ),
        area,
    );
    if let OverlayState::ApiKeyPrompt { value, cursor, .. } = overlay {
        let cursor_x = area.x + 2 + (*cursor as u16).min(area.width.saturating_sub(4));
        let cursor_y = area.y + height.saturating_sub(3);
        frame.set_cursor_position((cursor_x, cursor_y));
        if value.is_empty() && area.width > 6 {
            frame.render_widget(
                Paragraph::new("Paste code here if prompted >")
                    .style(Style::default().add_modifier(Modifier::DIM)),
                Rect {
                    x: area.x + 2,
                    y: cursor_y,
                    width: area.width.saturating_sub(4),
                    height: 1,
                },
            );
        }
    }
}

fn onboarding_body_lines(overlay: &OverlayState) -> Vec<Line<'static>> {
    let (title, subtitle, note, rows, footer) = match overlay {
        OverlayState::ThemePicker { .. } => (
            "Let's get started.",
            "Choose the text style that looks best with your terminal.",
            Some("You can run /theme later."),
            overlay_rows(overlay),
            "Enter to continue",
        ),
        OverlayState::ProviderPicker { .. } => (
            "Choose a provider.",
            "Pick where Puffer should sign in and load models from.",
            None,
            overlay_rows(overlay),
            "Enter to continue",
        ),
        OverlayState::AuthPicker { provider_id, .. } => (
            "Select login method.",
            "Choose how to connect your account for this provider.",
            Some(provider_id.as_str()),
            overlay_rows(overlay),
            "Enter to continue · Esc to go back",
        ),
        OverlayState::ModelPicker {
            provider_id,
            onboarding: true,
            ..
        } => (
            "Select model.",
            "Only models from the selected provider are shown here.",
            Some(provider_id.as_str()),
            overlay_rows(overlay),
            "Enter to confirm · Esc to go back",
        ),
        OverlayState::ApiKeyPrompt { provider_id, value, .. } => {
            let key_line = format!("> {}", masked_secret(value));
            return vec![
                Line::from(Span::styled(
                    "Let's get started.",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::default(),
                Line::from("Paste an API key."),
                Line::from(Span::styled(
                    format!("{provider_id} will use this key for API requests."),
                    Style::default().add_modifier(Modifier::DIM),
                )),
                Line::default(),
                Line::from(key_line),
                Line::default(),
                Line::from(Span::styled(
                    "Enter to continue · Esc to go back",
                    Style::default().add_modifier(Modifier::DIM),
                )),
            ];
        }
        _ => return Vec::new(),
    };

    let mut lines = vec![
        Line::from(Span::styled(
            "Puffer Code",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::default(),
        Line::from(Span::styled(
            title.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            subtitle.to_string(),
            Style::default().add_modifier(Modifier::DIM),
        )),
    ];
    if let Some(note) = note {
        lines.push(Line::from(Span::styled(
            note.to_string(),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    lines.push(Line::default());
    for row in rows {
        let marker = if row.selected { "› " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(
                marker.to_string(),
                if row.selected {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            ),
            Span::styled(
                row.text,
                if row.selected {
                    Style::default()
                        .add_modifier(Modifier::BOLD)
                        .add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                },
            ),
        ]));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        footer.to_string(),
        Style::default().add_modifier(Modifier::DIM),
    )));
    lines
}

#[cfg(test)]
mod tests;
