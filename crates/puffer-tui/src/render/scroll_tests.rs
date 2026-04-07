use super::*;
use puffer_core::MessageRole;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

#[test]
fn render_scrolls_top_panel_with_transcript() {
    let backend = TestBackend::new(80, 14);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = tests::sample_state();
    for index in 0..18 {
        state.push_message(
            MessageRole::Assistant,
            &format!("assistant line {index}\nthis line keeps the transcript tall enough to scroll"),
        );
    }
    let resources = tests::sample_resources();
    let providers = tests::sample_providers();
    let auth_store = tests::sample_auth_store();

    terminal
        .draw(|frame| {
            set_follow_output(false);
            render(
                frame,
                &state,
                &resources,
                &providers,
                &auth_store,
                "",
                0,
                0,
                24,
                &tests::sample_commands(),
            );
            set_follow_output(true);
        })
        .unwrap();

    let rendered = tests::terminal_view(&terminal);
    assert!(!rendered.contains("╭ Puffer Code"));
    assert!(rendered.contains("assistant line"));
}
