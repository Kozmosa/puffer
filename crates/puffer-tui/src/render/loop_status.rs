use crate::state::{LoopKind, LoopState, LoopStatus};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// Renders the persistent loop/optimization status box.
pub(crate) fn render_loop_status_box(frame: &mut Frame<'_>, area: Rect, loop_state: &LoopState) {
    let (icon, title) = match &loop_state.kind {
        LoopKind::Loop => ("⟳", "Loop"),
        LoopKind::Maximize(_) => ("▲", "Optimize"),
        LoopKind::Minimize(_) => ("▼", "Optimize"),
    };

    let border_color = match &loop_state.status {
        LoopStatus::Running => Color::Cyan,
        LoopStatus::WaitingInterval => Color::Yellow,
        LoopStatus::Paused => Color::Gray,
        LoopStatus::Completed(_) => Color::Green,
    };

    let block = Block::default()
        .title(format!(" {icon} {title} "))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut spans: Vec<Span<'_>> = Vec::new();

    // Kind + metric label
    match &loop_state.kind {
        LoopKind::Loop => {
            spans.push(Span::styled(
                format!("{icon} loop "),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            let prompt_preview: String = loop_state.prompt.chars().take(30).collect();
            spans.push(Span::styled(
                format!("\"{prompt_preview}\""),
                Style::default().fg(Color::White),
            ));
        }
        LoopKind::Maximize(m) | LoopKind::Minimize(m) => {
            let verb = if matches!(loop_state.kind, LoopKind::Maximize(_)) {
                "maximize"
            } else {
                "minimize"
            };
            spans.push(Span::styled(
                format!("{icon} {verb} "),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                format!("\"{m}\""),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }

    // Iteration counter
    spans.push(Span::styled(
        format!(
            "  iter {}/{}",
            loop_state.iteration, loop_state.max_iterations
        ),
        Style::default().fg(Color::White),
    ));

    let line1 = Line::from(spans);

    // Second line: metric history + status
    let mut line2_spans: Vec<Span<'_>> = Vec::new();

    if !loop_state.target_history.is_empty() {
        let maximize = matches!(loop_state.kind, LoopKind::Maximize(_));
        let history = &loop_state.target_history;

        line2_spans.push(Span::raw("  "));
        for (i, value) in history.iter().enumerate() {
            let color = if i == 0 {
                Color::White
            } else {
                let prev = history[i - 1];
                let improving = if maximize {
                    *value > prev
                } else {
                    *value < prev
                };
                if (*value - prev).abs() < f64::EPSILON {
                    Color::Yellow
                } else if improving {
                    Color::Green
                } else {
                    Color::Red
                }
            };
            if i > 0 {
                line2_spans.push(Span::styled(" → ", Style::default().fg(Color::DarkGray)));
            }
            line2_spans.push(Span::styled(
                format!("{value:.4}"),
                Style::default().fg(color),
            ));
        }

        // Show delta from last two values
        if history.len() >= 2 {
            let last = history[history.len() - 1];
            let prev = history[history.len() - 2];
            let delta = last - prev;
            let (arrow, color) = if delta.abs() < f64::EPSILON {
                ("=", Color::Yellow)
            } else if delta > 0.0 {
                (
                    "↑",
                    if matches!(loop_state.kind, LoopKind::Maximize(_)) {
                        Color::Green
                    } else {
                        Color::Red
                    },
                )
            } else {
                (
                    "↓",
                    if matches!(loop_state.kind, LoopKind::Minimize(_)) {
                        Color::Green
                    } else {
                        Color::Red
                    },
                )
            };
            line2_spans.push(Span::styled(
                format!("  ({arrow}{delta:+.4})"),
                Style::default().fg(color),
            ));
        }
    }

    // Status indicator
    let status_string = match &loop_state.status {
        LoopStatus::Running => "● Running".to_string(),
        LoopStatus::WaitingInterval => loop_state
            .next_fire
            .map(|t| {
                let now = std::time::Instant::now();
                if t > now {
                    let secs = (t - now).as_secs();
                    format!("◌ Waiting {secs}s")
                } else {
                    "◌ Firing...".to_string()
                }
            })
            .unwrap_or_else(|| "◌ Waiting".to_string()),
        LoopStatus::Paused => "⏸ Paused".to_string(),
        LoopStatus::Completed(reason) => format!("✓ Done: {reason}"),
    };
    let status_color = match &loop_state.status {
        LoopStatus::Running => Color::Green,
        LoopStatus::WaitingInterval => Color::Yellow,
        LoopStatus::Paused => Color::Gray,
        LoopStatus::Completed(_) => Color::Green,
    };
    line2_spans.push(Span::raw("  "));
    line2_spans.push(Span::styled(
        status_string,
        Style::default().fg(status_color),
    ));

    let line2 = Line::from(line2_spans);

    let text = ratatui::text::Text::from(vec![line1, line2]);
    let paragraph = Paragraph::new(text);
    frame.render_widget(paragraph, inner);
}

/// Returns the height needed for the loop status box (0 if no loop active).
pub(crate) fn loop_status_height(loop_state: &Option<LoopState>) -> u16 {
    if loop_state.is_some() {
        4 // border top + 2 content lines + border bottom
    } else {
        0
    }
}
