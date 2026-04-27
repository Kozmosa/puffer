/// Returns whether a session without a user-set title should be auto-titled.
pub(crate) fn should_auto_title(display_name: Option<&str>, has_user_message: bool) -> bool {
    display_name
        .map(str::trim)
        .map(str::is_empty)
        .unwrap_or(true)
        && !has_user_message
}

/// Builds a deterministic session title from the first user message.
pub(crate) fn title_from_first_message(message: &str) -> Option<String> {
    let collapsed = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return None;
    }

    const MAX_CHARS: usize = 60;
    let mut title = trimmed.chars().take(MAX_CHARS + 1).collect::<String>();
    if title.chars().count() > MAX_CHARS {
        title = title.chars().take(MAX_CHARS).collect::<String>();
        let without_partial_word = title
            .rsplit_once(' ')
            .map(|(prefix, _)| prefix)
            .filter(|prefix| prefix.chars().count() >= 20)
            .unwrap_or(title.as_str())
            .trim_end()
            .to_string();
        title = format!("{without_partial_word}...");
    }

    Some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_title_requires_empty_name_and_no_user_messages() {
        assert!(should_auto_title(None, false));
        assert!(should_auto_title(Some("  "), false));
        assert!(!should_auto_title(Some("Named"), false));
        assert!(!should_auto_title(None, true));
    }

    #[test]
    fn title_from_first_message_collapses_whitespace() {
        assert_eq!(
            title_from_first_message("  Fix   the browser\nsession title  ").as_deref(),
            Some("Fix the browser session title")
        );
    }

    #[test]
    fn title_from_first_message_truncates_long_text() {
        assert_eq!(
            title_from_first_message(
                "Please investigate why the session title is never updated after sending a message"
            )
            .as_deref(),
            Some("Please investigate why the session title is never updated...")
        );
    }
}
