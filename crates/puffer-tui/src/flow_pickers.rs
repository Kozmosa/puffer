use puffer_config::ConfigPaths;
use puffer_core::{AppState, MessageRole};

pub(crate) fn rewind_picker_entries(state: &AppState) -> Vec<crate::ModelPickerEntry> {
    let mut entries = vec![crate::ModelPickerEntry {
        selector: "/rewind".to_string(),
        description: "Remove the latest rendered transcript item".to_string(),
        command: None,
    }];
    entries.extend(
        state
            .transcript
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .enumerate()
            .map(|(index, message)| crate::ModelPickerEntry {
                selector: format!("/rewind {}", index + 1),
                description: truncate_rewind_label(&message.text),
                command: None,
            }),
    );
    entries
}

fn truncate_rewind_label(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or("<empty>");
    if line.chars().count() <= 60 {
        line.to_string()
    } else {
        format!("{}...", line.chars().take(57).collect::<String>())
    }
}

pub(crate) fn memory_picker_entries(state: &AppState) -> Vec<crate::ModelPickerEntry> {
    let paths = ConfigPaths::discover(&state.cwd);
    [
        ("project", state.cwd.join("CLAUDE.md")),
        ("workspace", paths.workspace_config_dir.join("memory.md")),
        ("user", paths.user_config_dir.join("memory.md")),
    ]
    .into_iter()
    .map(|(scope, path)| crate::ModelPickerEntry {
        selector: format!("/memory open {scope}"),
        description: format!(
            "{} ({})",
            path.display(),
            if path.exists() { "present" } else { "new" }
        ),
        command: None,
    })
    .collect()
}
