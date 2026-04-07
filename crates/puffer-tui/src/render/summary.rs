use crate::popup::popup_rows;
use puffer_core::{AppState, CommandSpec, MessageRole, RenderedMessage};
use puffer_provider_registry::{AuthStore, StoredCredential};
use puffer_resources::LoadedResources;
use puffer_tools::ToolRegistry;
use ratatui::text::{Line, Span};
use std::path::Path;

const COMPACT_TOP_PANEL_BREAKPOINT: u16 = 84;

/// Builds the two columns rendered inside the top information box.
pub(super) fn top_panel_columns(
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
) -> [Vec<Line<'static>>; 2] {
    let left = FieldFormatter::from_labels(["Mascot", "User", "Provider", "Mode"]);
    let right = FieldFormatter::from_labels(["Session", "Model", "Directory", "Activity"]);
    let mascot = if state.config.mascot.enabled {
        format!("{} on duty", state.config.mascot.display_name)
    } else {
        "Mascot disabled".to_string()
    };
    let user = account_header_line(state, auth_store).unwrap_or_else(|| {
        let provider = current_provider(state);
        match auth_status(state, auth_store) {
            "api-key" => format!("{provider} via API key"),
            "oauth" => format!("{provider} via OAuth"),
            "missing" => format!("{provider} auth missing"),
            _ => "No active account".to_string(),
        }
    });
    let mut mode = vec![format!("effort {}", truncate(&state.effort_level, 14))];
    if state.fast_mode {
        mode.push("fast".to_string());
    }
    if state.vim_mode {
        mode.push("vim".to_string());
    }
    let remote = remote_label(state);
    let activity = if remote == "local" {
        format!(
            "{} messages · {} workdirs · sandbox {}",
            state.transcript.len(),
            state.working_dirs.len(),
            truncate(&state.sandbox_mode, 18),
        )
    } else {
        format!(
            "{} messages · {} workdirs · {}",
            state.transcript.len(),
            state.working_dirs.len(),
            truncate(&remote, 18),
        )
    };
    let tools = tool_status(tool_registry).executable;

    [
        vec![
            left.line(
                "Mascot",
                vec![Span::styled(
                    truncate(&mascot, 32),
                    ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::BOLD),
                )],
            ),
            left.line("User", vec![Span::raw(truncate(&user, 40))]),
            left.line(
                "Provider",
                vec![Span::raw(format!(
                    "{} · auth {}",
                    truncate(current_provider(state), 20),
                    auth_status(state, auth_store),
                ))],
            ),
            left.line("Mode", vec![Span::raw(mode.join(" · "))]),
        ],
        vec![
            right.line(
                "Session",
                vec![Span::raw(format!(
                    "{} · {}",
                    truncate(&session_name(state), 22),
                    short_id(&state.session.id.to_string()),
                ))],
            ),
            right.line(
                "Model",
                vec![Span::raw(format!(
                    "{} · tools {tools}/{}",
                    truncate(current_model(state), 24),
                    resources.tools.len(),
                ))],
            ),
            right.line(
                "Directory",
                vec![Span::raw(truncate(&path_tail(&state.cwd), 34))],
            ),
            right.line("Activity", vec![Span::raw(activity)]),
        ],
    ]
}

/// Builds the stacked summary rows used when the viewport is narrow.
pub(super) fn top_panel_compact_lines(
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
) -> Vec<Line<'static>> {
    let [left, right] = top_panel_columns(state, resources, auth_store, tool_registry);
    left.into_iter().chain(right).collect()
}

/// Returns the outer height needed for the top information box.
pub(super) fn top_panel_height(
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
    width: u16,
) -> u16 {
    if width < COMPACT_TOP_PANEL_BREAKPOINT {
        top_panel_compact_lines(state, resources, auth_store, tool_registry).len() as u16 + 2
    } else {
        let [left, right] = top_panel_columns(state, resources, auth_store, tool_registry);
        left.len().max(right.len()) as u16 + 2
    }
}

/// Builds the primary footer status line shown above the composer.
pub(super) fn status_primary_line(
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
) -> String {
    format!(
        "{} · {} · auth {} · tools {}/{}",
        truncate(current_provider(state), 18),
        truncate(current_model(state), 28),
        auth_status(state, auth_store),
        tool_status(tool_registry).executable,
        resources.tools.len(),
    )
}

/// Builds the secondary footer line shown below the composer prompt.
pub(super) fn status_secondary_line(
    state: &AppState,
    resources: &LoadedResources,
    tool_registry: &ToolRegistry,
) -> String {
    let mut line = format!(
        "{} · shell {} · prompts {} · {} workdirs",
        truncate(&path_tail(&state.cwd), 18),
        shell_activity(&state.transcript).total_runs,
        resources.prompts.len(),
        state.working_dirs.len(),
    );
    let remote = remote_label(state);
    if remote != "local" {
        line.push_str(&format!(" · {}", truncate(&remote, 18)));
    }
    if state.statusline_enabled {
        line.push_str(&format!(" · sandbox {}", truncate(&state.sandbox_mode, 18)));
    }
    if tool_status(tool_registry).executable == 0 {
        line.push_str(" · no tools");
    }
    line
}

/// Builds a compact one-line session summary for medium and narrow footers.
pub(super) fn status_compact_line(
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
) -> String {
    format!(
        "{} · {} · auth {} · tools {}/{} · {}",
        truncate(current_provider(state), 14),
        truncate(current_model(state), 22),
        auth_status(state, auth_store),
        tool_status(tool_registry).executable,
        resources.tools.len(),
        truncate(&path_tail(&state.cwd), 16),
    )
}

/// Returns true when the top info box should stack instead of using two columns.
pub(super) fn use_compact_top_panel(width: u16) -> bool {
    width < COMPACT_TOP_PANEL_BREAKPOINT
}

/// Builds the contextual composer hint line.
pub(super) fn hint_line(input: &str, commands: &[CommandSpec]) -> String {
    if input.starts_with('/') {
        let rows = popup_rows(input, commands);
        let best = rows
            .first()
            .map(|command| format!("/{}", command.name))
            .unwrap_or_else(|| "<none>".to_string());
        return format!(
            "slash {} · {} matches · best {} · Enter submits · Esc clears",
            truncate(input, 18),
            rows.len(),
            best,
        );
    }

    "? for shortcuts · /help · /review · !pwd".to_string()
}

/// Flattens the top-box summary into test-friendly lines.
#[cfg(test)]
pub(super) fn header_lines(
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
) -> Vec<Line<'static>> {
    let [left, right] = top_panel_columns(state, resources, auth_store, tool_registry);
    let mut lines = vec![Line::from("Puffer Code")];
    lines.extend(left);
    lines.push(Line::default());
    lines.extend(right);
    lines
}

/// Builds the serialized session summary used in render tests.
#[cfg(test)]
pub(super) fn session_lines(state: &AppState) -> Vec<String> {
    let parent = state
        .session
        .parent_session_id
        .map(|value| short_id(&value.to_string()))
        .unwrap_or_else(|| "root".to_string());
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
        format!("Note: {}", state.session.note.as_deref().unwrap_or("-")),
    ]
}

/// Builds the test-only footer summary lines.
#[cfg(test)]
pub(super) fn footer_lines(
    state: &AppState,
    resources: &LoadedResources,
    auth_store: &AuthStore,
    tool_registry: &ToolRegistry,
    input: &str,
    commands: &[CommandSpec],
) -> Vec<Line<'static>> {
    vec![
        Line::from(status_primary_line(
            state,
            resources,
            auth_store,
            tool_registry,
        )),
        Line::from(status_secondary_line(state, resources, tool_registry)),
        Line::from(hint_line(input, commands)),
    ]
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ToolStatus {
    executable: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ShellActivity {
    total_runs: usize,
    last_command: Option<String>,
}

struct FieldFormatter {
    label_width: usize,
}

impl FieldFormatter {
    fn from_labels<const N: usize>(labels: [&str; N]) -> Self {
        Self {
            label_width: labels.iter().map(|label| label.len()).max().unwrap_or(0),
        }
    }

    fn line(&self, label: &str, value: Vec<Span<'static>>) -> Line<'static> {
        let mut spans = vec![Span::styled(
            format!("{label:width$}  ", width = self.label_width),
            ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM),
        )];
        spans.extend(value);
        Line::from(spans)
    }
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

fn account_header_line(state: &AppState, auth_store: &AuthStore) -> Option<String> {
    let provider_id = state.current_provider.as_deref()?;
    let StoredCredential::OAuth(credential) = auth_store.get(provider_id)? else {
        return None;
    };
    let mut parts = Vec::new();
    if let Some(email) = credential.email.as_deref() {
        parts.push(email.to_string());
    }
    if let Some(plan_type) = credential.plan_type.as_deref() {
        parts.push(format!("plan {}", format_metadata_value(plan_type)));
    }
    if let Some(organization_id) = credential.organization_id.as_deref() {
        parts.push(format!("org {}", organization_id));
    } else if let Some(account_id) = credential.account_id.as_deref() {
        parts.push(format!("acct {}", account_id));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn format_metadata_value(value: &str) -> String {
    value
        .split(['-', '_'])
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => {
                    let mut word = String::new();
                    word.extend(first.to_uppercase());
                    word.push_str(chars.as_str());
                    word
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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

#[cfg(test)]
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

fn tool_status(tool_registry: &ToolRegistry) -> ToolStatus {
    let mut status = ToolStatus::default();
    for _tool in tool_registry.tools() {
        status.executable += 1;
    }
    status
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
