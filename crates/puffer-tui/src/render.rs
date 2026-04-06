use crate::markdown::render_markdown;
use crate::popup::popup_rows;
use puffer_core::{AppState, CommandSpec, MessageRole};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

/// Renders the current application frame.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    state: &AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    input: &str,
    commands: &[CommandSpec],
) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
        .split(layout[1]);

    let header = Paragraph::new(format!(
        "Puffer Code  provider={}  model={}  theme={}  session={}",
        state.current_provider.as_deref().unwrap_or("unset"),
        state.current_model.as_deref().unwrap_or("unset"),
        state.config.theme,
        state.session.id
    ))
    .block(Block::default().title("Header").borders(Borders::ALL));
    frame.render_widget(header, layout[0]);

    let transcript = if state.transcript.is_empty() {
        Text::from("No transcript yet. Type a prompt or a slash command.")
    } else {
        Text::from(
            state
                .transcript
                .iter()
                .flat_map(|message| {
                    let prefix = match message.role {
                        MessageRole::User => "You",
                        MessageRole::Assistant => "Puffer",
                        MessageRole::System => "System",
                    };
                    let rendered = render_markdown(&message.text);
                    rendered
                        .lines
                        .into_iter()
                        .enumerate()
                        .map(move |(index, line)| {
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
                })
                .collect::<Vec<_>>(),
        )
    };

    let transcript_widget = Paragraph::new(transcript)
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Transcript").borders(Borders::ALL));
    frame.render_widget(transcript_widget, body[0]);

    let current_provider = state.current_provider.as_deref().unwrap_or("<unset>");
    let inspector_lines = vec![
        format!("Provider: {current_provider}"),
        format!(
            "Auth stored: {}",
            if auth_store.has_auth(current_provider) {
                "yes"
            } else {
                "no"
            }
        ),
        format!("Model: {}", state.current_model.as_deref().unwrap_or("<unset>")),
        format!("Theme: {}", state.config.theme),
        format!("Prompt color: {}", state.prompt_color),
        format!("Effort: {}", state.effort_level),
        format!("Fast mode: {}", if state.fast_mode { "on" } else { "off" }),
        format!("Sandbox: {}", state.sandbox_mode),
        format!("Transcript: {} msgs", state.transcript.len()),
        format!("Providers: {}", providers.providers().count()),
        format!("Tools: {}", resources.tools.len()),
        format!("Skills: {}", resources.skills.len()),
        format!("Plugins: {}", resources.plugins.len()),
        format!("MCP: {}", resources.mcp_servers.len()),
        format!("IDEs: {}", resources.ides.len()),
    ];
    let inspector = Paragraph::new(inspector_lines.join("\n"))
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Inspector").borders(Borders::ALL));
    frame.render_widget(inspector, body[1]);

    let input_widget = Paragraph::new(input.to_string())
        .block(Block::default().title("Input").borders(Borders::ALL));
    frame.render_widget(input_widget, layout[2]);

    let footer = Paragraph::new(
        "Enter submits  Esc clears  Ctrl+C exits  / commands  tools via /permissions  auth via /login",
    )
        .block(Block::default().title("Footer").borders(Borders::ALL));
    frame.render_widget(footer, layout[3]);

    if input.starts_with('/') {
        render_command_popup(frame, body[0], input, commands);
    }
}

fn render_command_popup(
    frame: &mut Frame<'_>,
    transcript_area: Rect,
    input: &str,
    commands: &[CommandSpec],
) {
    let matching = popup_rows(input, commands)
        .into_iter()
        .map(|command| {
            let alias_suffix = if command.aliases.is_empty() {
                String::new()
            } else {
                format!(" [{}]", command.aliases.join(", "))
            };
            ListItem::new(format!(
                "/{:<16} {}{}",
                command.name, command.description, alias_suffix
            ))
            .style(Style::default().add_modifier(Modifier::BOLD))
        })
        .collect::<Vec<_>>();

    let popup_area = Rect {
        x: transcript_area.x + 2,
        y: transcript_area.y + 2,
        width: transcript_area.width.saturating_sub(4).min(80),
        height: matching.len() as u16 + 2,
    };
    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        List::new(matching).block(Block::default().title("Commands").borders(Borders::ALL)),
        popup_area,
    );
}
