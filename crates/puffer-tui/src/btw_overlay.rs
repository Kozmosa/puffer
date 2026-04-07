use crate::markdown::render_markdown;
use crate::OverlayState;
use puffer_core::{execute_side_question, AppState};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::thread;

const MIN_OVERLAY_WIDTH: u16 = 42;
const MAX_OVERLAY_WIDTH: u16 = 96;

/// Stores the mutable interactive `/btw` overlay state.
#[derive(Clone)]
pub(crate) struct BtwOverlay {
    shared: Arc<Mutex<BtwOverlayState>>,
}

#[derive(Debug, Clone)]
struct BtwOverlayState {
    question: String,
    view: BtwOverlayView,
    scroll: u16,
    generation: u64,
}

#[derive(Debug, Clone)]
enum BtwOverlayView {
    Loading,
    Ready(String),
    Error(String),
}

#[derive(Debug, Clone)]
struct BtwOverlaySnapshot {
    question: String,
    view: BtwOverlayView,
    scroll: u16,
}

impl BtwOverlay {
    /// Builds the current `/btw` overlay and starts the side-question request.
    pub(crate) fn open(
        state: &AppState,
        resources: &LoadedResources,
        providers: &ProviderRegistry,
        auth_store: &AuthStore,
        question: &str,
    ) -> OverlayState {
        let overlay = BtwOverlay {
            shared: Arc::new(Mutex::new(BtwOverlayState {
                question: question.trim().to_string(),
                view: BtwOverlayView::Loading,
                scroll: 0,
                generation: 0,
            })),
        };
        overlay.start_load(
            state.clone(),
            resources.clone(),
            providers.clone(),
            auth_store.clone(),
        );
        OverlayState::Btw(overlay)
    }

    /// Scrolls the overlay upward by one row.
    pub(crate) fn scroll_up(&self) {
        if let Ok(mut state) = self.shared.lock() {
            state.scroll = state.scroll.saturating_sub(1);
        }
    }

    /// Scrolls the overlay downward by one row.
    pub(crate) fn scroll_down(&self) {
        if let Ok(mut state) = self.shared.lock() {
            state.scroll = state.scroll.saturating_add(1);
        }
    }

    /// Scrolls the overlay upward by one page.
    pub(crate) fn page_up(&self) {
        if let Ok(mut state) = self.shared.lock() {
            state.scroll = state.scroll.saturating_sub(10);
        }
    }

    /// Scrolls the overlay downward by one page.
    pub(crate) fn page_down(&self) {
        if let Ok(mut state) = self.shared.lock() {
            state.scroll = state.scroll.saturating_add(10);
        }
    }

    fn start_load(
        &self,
        state: AppState,
        resources: LoadedResources,
        providers: ProviderRegistry,
        auth_store: AuthStore,
    ) {
        let (question, generation) = if let Ok(mut overlay) = self.shared.lock() {
            overlay.generation = overlay.generation.saturating_add(1);
            overlay.view = BtwOverlayView::Loading;
            overlay.scroll = 0;
            (overlay.question.clone(), overlay.generation)
        } else {
            return;
        };
        let shared = Arc::clone(&self.shared);
        thread::spawn(move || {
            let mut auth_store = auth_store;
            let view = match execute_side_question(
                &state,
                &resources,
                &providers,
                &mut auth_store,
                &question,
            ) {
                Ok(turn) => BtwOverlayView::Ready(turn.assistant_text),
                Err(error) => BtwOverlayView::Error(error.to_string()),
            };
            let Ok(mut overlay) = shared.lock() else {
                return;
            };
            if overlay.generation == generation {
                overlay.view = view;
                overlay.scroll = 0;
            }
        });
    }

    fn snapshot(&self) -> BtwOverlaySnapshot {
        self.shared
            .lock()
            .map(|state| BtwOverlaySnapshot {
                question: state.question.clone(),
                view: state.view.clone(),
                scroll: state.scroll,
            })
            .unwrap_or(BtwOverlaySnapshot {
                question: String::new(),
                view: BtwOverlayView::Error("BTW overlay is unavailable.".to_string()),
                scroll: 0,
            })
    }

    #[cfg(test)]
    pub(crate) fn ready_for_test(question: &str, response: &str) -> Self {
        Self {
            shared: Arc::new(Mutex::new(BtwOverlayState {
                question: question.to_string(),
                view: BtwOverlayView::Ready(response.to_string()),
                scroll: 0,
                generation: 0,
            })),
        }
    }
}

impl PartialEq for BtwOverlay {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
}

impl Eq for BtwOverlay {}

impl fmt::Debug for BtwOverlay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("BtwOverlay").finish_non_exhaustive()
    }
}

/// Renders the side-question overlay.
pub(crate) fn render_btw_overlay(frame: &mut Frame<'_>, viewport: Rect, overlay: &BtwOverlay) {
    let snapshot = overlay.snapshot();
    let width = viewport
        .width
        .saturating_sub(8)
        .clamp(MIN_OVERLAY_WIDTH, MAX_OVERLAY_WIDTH);
    let area = Rect {
        x: viewport.x + viewport.width.saturating_sub(width) / 2,
        y: viewport.y + 1,
        width,
        height: viewport.height.saturating_sub(2).max(8),
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(overlay_text(&snapshot))
            .scroll((snapshot.scroll, 0))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title("BTW")
                    .borders(Borders::ALL)
                    .border_style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
            ),
        area,
    );
}

fn overlay_text(snapshot: &BtwOverlaySnapshot) -> Text<'static> {
    let mut lines = vec![Line::from(vec![
        Span::styled(
            "/btw ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            snapshot.question.clone(),
            Style::default().add_modifier(Modifier::DIM),
        ),
    ])];
    lines.push(Line::default());
    match &snapshot.view {
        BtwOverlayView::Loading => {
            lines.push(Line::from(Span::styled(
                "Answering...",
                Style::default().fg(Color::Yellow),
            )));
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                "Esc closes",
                Style::default().add_modifier(Modifier::DIM),
            )));
        }
        BtwOverlayView::Ready(response) => {
            lines.extend(render_markdown(response).lines);
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                "Up/Down scroll · PgUp/PgDn page · Space, Enter, or Esc closes",
                Style::default().add_modifier(Modifier::DIM),
            )));
        }
        BtwOverlayView::Error(message) => {
            lines.push(Line::from(Span::styled(
                message.clone(),
                Style::default().fg(Color::Red),
            )));
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                "Enter or Esc closes",
                Style::default().add_modifier(Modifier::DIM),
            )));
        }
    }
    Text::from(lines)
}
