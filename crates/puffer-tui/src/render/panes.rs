use super::prompt_border_style;
use puffer_core::{AppState, CommandSpec};
use puffer_resources::LoadedResources;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::symbols::border;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

const HOME_WIDE_BREAKPOINT: u16 = 96;
const HELP_WIDE_BREAKPOINT: u16 = 120;
const FEATURED_HELP_COMMANDS: [&str; 10] = [
    "help",
    "review",
    "resume",
    "login",
    "model",
    "agents",
    "usage",
    "doctor",
    "config",
    "skills",
];

/// Renders the empty-session welcome body with width-aware cards.
pub(super) fn render_empty_state(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let card_width = area.width.saturating_sub(6).clamp(38, 104);
    let wide = area.width >= HOME_WIDE_BREAKPOINT && area.height >= 12;
    let desired_height = if wide { 12 } else { 10 };
    let card_height = desired_height.min(area.height.saturating_sub(1).max(6));
    let card_area = Rect {
        x: area.x + area.width.saturating_sub(card_width) / 2,
        y: area.y + area.height.saturating_sub(card_height).min(2),
        width: card_width,
        height: card_height,
    };
    frame.render_widget(Clear, card_area);
    let block = Block::default()
        .title(" Puffer Code ")
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(prompt_border_style(state));
    frame.render_widget(&block, card_area);
    let inner = block.inner(card_area);

    if wide {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(58),
                Constraint::Length(1),
                Constraint::Percentage(42),
            ])
            .split(inner);
        frame.render_widget(
            Paragraph::new(Text::from(welcome_left_lines(state))).wrap(Wrap { trim: false }),
            columns[0],
        );
        frame.render_widget(
            Paragraph::new(Text::from(vertical_separator(columns[1].height))),
            columns[1],
        );
        frame.render_widget(
            Paragraph::new(Text::from(welcome_right_lines(state))).wrap(Wrap { trim: false }),
            columns[2],
        );
    } else {
        frame.render_widget(
            Paragraph::new(Text::from(welcome_compact_lines(state)))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false }),
            inner,
        );
    }
}

/// Renders the help pane using a responsive command summary layout.
pub(super) fn render_help_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    commands: &[CommandSpec],
    resources: &LoadedResources,
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(prompt_border_style(state));
    frame.render_widget(&block, area);
    let inner = block.inner(area);
    let content = Rect {
        x: inner.x.saturating_add(1),
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };

    if content.width >= HELP_WIDE_BREAKPOINT && content.height >= 20 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(62),
                Constraint::Length(1),
                Constraint::Percentage(38),
            ])
            .split(content);
        frame.render_widget(
            Paragraph::new(Text::from(help_command_lines(
                commands,
                columns[0].width,
                columns[0].height,
            )))
            .wrap(Wrap { trim: false }),
            columns[0],
        );
        frame.render_widget(
            Paragraph::new(Text::from(vertical_separator(columns[1].height))),
            columns[1],
        );
        frame.render_widget(
            Paragraph::new(Text::from(help_side_lines(resources)))
                .wrap(Wrap { trim: false }),
            columns[2],
        );
    } else {
        frame.render_widget(
            Paragraph::new(Text::from(help_compact_lines(
                commands,
                resources,
                content.width,
                content.height,
            )))
            .wrap(Wrap { trim: false }),
            content,
        );
    }
}

fn welcome_left_lines(state: &AppState) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "Welcome to Puffer Code",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(mascot_status(state)),
        Line::from(Span::styled(
            current_model_label(state),
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            path_tail(&state.cwd.display().to_string(), 48),
            Style::default().add_modifier(Modifier::DIM),
        )),
    ]
}

fn welcome_right_lines(state: &AppState) -> Vec<Line<'static>> {
    let recent = if state.transcript.is_empty() {
        "No recent activity yet.".to_string()
    } else {
        format!("{} messages in this session.", state.transcript.len())
    };
    vec![
        Line::from(Span::styled(
            "Tips to get started",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from("/help   Browse common commands"),
        Line::from("/review Review the current worktree"),
        Line::from("/login  Switch provider auth"),
        Line::default(),
        Line::from(Span::styled(
            "Recent activity",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(recent, Style::default().add_modifier(Modifier::DIM))),
    ]
}

fn welcome_compact_lines(state: &AppState) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "Welcome to Puffer Code",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(mascot_status(state)),
        Line::from(Span::styled(
            current_model_label(state),
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            path_tail(&state.cwd.display().to_string(), 38),
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::default(),
        Line::from(Span::styled(
            "Review changes, ask a question, or type /",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "? for shortcuts · /help to begin",
            Style::default().add_modifier(Modifier::DIM),
        )),
    ]
}

fn help_command_lines(commands: &[CommandSpec], width: u16, height: u16) -> Vec<Line<'static>> {
    let row_width = usize::from(width.saturating_sub(1));
    let mut lines = vec![
        Line::from(Span::styled(
            format!("Puffer Code v{}  Supported commands", env!("CARGO_PKG_VERSION")),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(
            "Core commands for common coding workflows. Use /skills for loaded /skill:<name> entries.",
        ),
        Line::default(),
    ];
    let entries = featured_help_commands(commands);
    let column_width = row_width.saturating_sub(3) / 2;
    let row_capacity = usize::from(height.saturating_sub(6)).max(1);
    let pairs = entries.chunks(2).take(row_capacity);
    for pair in pairs {
        let left = format_help_entry(pair[0], column_width);
        let right = pair
            .get(1)
            .map(|command| format_help_entry(command, column_width))
            .unwrap_or_default();
        lines.push(Line::from(format!(
            "{:<left_width$}   {}",
            left,
            right,
            left_width = column_width
        )));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "Esc to cancel",
        Style::default().add_modifier(Modifier::DIM),
    )));
    lines
}

fn help_side_lines(resources: &LoadedResources) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "Tips to get started",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from("Review changes, ask a question, or type /."),
        Line::from("/review inspects the current worktree."),
        Line::from("/resume opens the saved-session picker."),
        Line::from("/login changes provider auth."),
        Line::default(),
        Line::from(Span::styled(
            "Resources",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "prompts {} · tools {} · skills {}",
            resources.prompts.len(),
            resources.tools.len(),
            resources.skills.len(),
        )),
        Line::from(format!(
            "plugins {} · mcp {} · ides {}",
            resources.plugins.len(),
            resources.mcp_servers.len(),
            resources.ides.len(),
        )),
        Line::default(),
        Line::from(Span::styled(
            "? for shortcuts",
            Style::default().add_modifier(Modifier::DIM),
        )),
    ]
}

fn help_compact_lines(
    commands: &[CommandSpec],
    resources: &LoadedResources,
    width: u16,
    height: u16,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "Supported commands",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    let entry_width = usize::from(width);
    let reserve_footer = if height >= 9 { 2 } else { 1 };
    let max_entries = usize::from(height.saturating_sub(reserve_footer + 1)).max(1);
    for command in featured_help_commands(commands).into_iter().take(max_entries) {
        lines.push(Line::from(format_help_entry(command, entry_width)));
    }
    if height >= 9 {
        lines.push(Line::from(Span::styled(
            format!(
                "Resources: prompts {} · tools {} · skills {}",
                resources.prompts.len(),
                resources.tools.len(),
                resources.skills.len(),
            ),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    lines.push(Line::from(Span::styled(
        "Esc to cancel",
        Style::default().add_modifier(Modifier::DIM),
    )));
    lines
}

fn featured_help_commands(commands: &[CommandSpec]) -> Vec<&CommandSpec> {
    let mut featured = FEATURED_HELP_COMMANDS
        .iter()
        .filter_map(|name| commands.iter().find(|command| command.name == *name))
        .collect::<Vec<_>>();
    if featured.is_empty() {
        featured.extend(commands.iter().take(10));
    }
    featured
}

fn format_help_entry(command: &CommandSpec, max_width: usize) -> String {
    if max_width <= 8 {
        return truncate(&format!("/{}", command.name), max_width);
    }
    let name = format!("/{}", command.name);
    let name_width = 12.min(max_width.saturating_sub(2));
    let description_width = max_width.saturating_sub(name_width + 2);
    format!(
        "{:<name_width$} {}",
        truncate(&name, name_width),
        truncate(command.description, description_width)
    )
}

fn mascot_status(state: &AppState) -> String {
    if state.config.mascot.enabled {
        format!("{} on duty", state.config.mascot.display_name)
    } else {
        "Mascot disabled".to_string()
    }
}

fn current_model_label(state: &AppState) -> String {
    state.current_model
        .clone()
        .unwrap_or_else(|| "/model to choose a model".to_string())
}

fn vertical_separator(height: u16) -> Vec<Line<'static>> {
    (0..height)
        .map(|_| Line::from(Span::styled("│", Style::default().add_modifier(Modifier::DIM))))
        .collect()
}

fn path_tail(path: &str, max_chars: usize) -> String {
    truncate(path.rsplit('/').next().unwrap_or(path), max_chars)
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
