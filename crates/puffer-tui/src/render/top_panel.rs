use super::prompt_border_style;
use super::summary::{
    top_panel_columns, top_panel_compact_lines, top_panel_height, use_compact_top_panel,
};
use puffer_core::AppState;
use puffer_provider_registry::AuthStore;
use puffer_resources::LoadedResources;
use puffer_tools::ToolRegistry;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::symbols::border;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};
use ratatui::Frame;

/// Renders the fixed top panel used on non-scrollable surfaces such as home.
pub(super) fn render_fixed_top_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
) {
    let block = panel_block(state);
    frame.render_widget(&block, area);
    render_panel_body(
        frame.buffer_mut(),
        block.inner(area),
        state,
        resources,
        auth_store,
        tool_registry,
    );
}

/// Returns the top panel as rendered lines so it can participate in transcript scrolling.
pub(super) fn scrollable_top_panel_lines(
    width: u16,
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }

    let area = Rect {
        x: 0,
        y: 0,
        width,
        height: top_panel_height(state, resources, auth_store, tool_registry, width).max(1),
    };
    let mut buffer = Buffer::empty(area);
    let block = panel_block(state);
    let inner = block.inner(area);
    block.render(area, &mut buffer);
    render_panel_body(
        &mut buffer,
        inner,
        state,
        resources,
        auth_store,
        tool_registry,
    );
    buffer_lines(&buffer, area)
}

fn panel_block(state: &AppState) -> Block<'static> {
    Block::default()
        .title(" Puffer Code ")
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(prompt_border_style(state))
}

fn render_panel_body(
    buffer: &mut Buffer,
    area: Rect,
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
) {
    if use_compact_top_panel(area.width.saturating_add(2)) {
        Paragraph::new(Text::from(top_panel_compact_lines(
            state,
            resources,
            auth_store,
            tool_registry,
        )))
        .wrap(Wrap { trim: false })
        .render(area, buffer);
        return;
    }

    let [left, right] = top_panel_columns(state, resources, auth_store, tool_registry);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(44),
            Constraint::Length(1),
            Constraint::Percentage(56),
        ])
        .split(area);
    Paragraph::new(Text::from(left))
        .wrap(Wrap { trim: false })
        .render(columns[0], buffer);
    if columns[1].width > 0 {
        Paragraph::new(Text::from(
            (0..columns[1].height)
                .map(|_| Line::from("│"))
                .collect::<Vec<_>>(),
        ))
        .render(columns[1], buffer);
    }
    Paragraph::new(Text::from(right))
        .wrap(Wrap { trim: false })
        .render(columns[2], buffer);
}

fn buffer_lines(buffer: &Buffer, area: Rect) -> Vec<Line<'static>> {
    (area.y..area.y + area.height)
        .map(|y| {
            let text = (area.x..area.x + area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>();
            Line::from(text)
        })
        .collect()
}
