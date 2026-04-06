use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

pub(super) fn render_tool_message(text: &str, expanded: bool) -> Option<Vec<Line<'static>>> {
    if expanded {
        return None;
    }
    let parsed = parse_tool_message(text)?;
    let mut lines = vec![Line::from(vec![
        Span::styled("⏺ ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("{} [{}]", friendly_tool_name(parsed.tool_id), parsed.status),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ])];

    if let Some(summary) = summarize_input(parsed.tool_id, parsed.input) {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default().add_modifier(Modifier::DIM)),
            Span::raw(summary),
        ]));
    }
    if let Some(preview) = output_preview(parsed.output) {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default().add_modifier(Modifier::DIM)),
            Span::styled(preview, Style::default().add_modifier(Modifier::DIM)),
        ]));
    }
    Some(lines)
}

struct ParsedToolMessage<'a> {
    tool_id: &'a str,
    status: &'a str,
    input: &'a str,
    output: Option<&'a str>,
}

fn parse_tool_message(text: &str) -> Option<ParsedToolMessage<'_>> {
    let (header, rest) = text.split_once('\n')?;
    let header = header.strip_prefix("Tool ")?;
    let (tool_id, status) = header.rsplit_once(" [")?;
    let status = status.strip_suffix(']')?;
    let input = rest.strip_prefix("input: ")?;
    let (input, output) = input
        .split_once('\n')
        .map(|(input, output)| (input, Some(output)))
        .unwrap_or((input, None));
    Some(ParsedToolMessage {
        tool_id,
        status,
        input,
        output,
    })
}

fn friendly_tool_name(tool_id: &str) -> String {
    match tool_id {
        "bash" => "Bash".to_string(),
        "read_file" => "Read File".to_string(),
        "write_file" => "Write File".to_string(),
        "replace_in_file" => "Edit File".to_string(),
        "search_text" => "Search Text".to_string(),
        "list_dir" => "List Directory".to_string(),
        "move_path" => "Move Path".to_string(),
        "remove_path" => "Remove Path".to_string(),
        "WebSearch" => "Web Search".to_string(),
        "WebFetch" => "Web Fetch".to_string(),
        "Task" => "Task".to_string(),
        "Agent" => "Agent".to_string(),
        other => other.replace('_', " "),
    }
}

fn summarize_input(tool_id: &str, input: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(input).ok();
    match tool_id {
        "bash" => extract_string(parsed.as_ref(), &["command"])
            .map(|command| format!("command: {command}")),
        "read_file" | "write_file" | "replace_in_file" => {
            extract_string(parsed.as_ref(), &["path"]).map(|path| format!("path: {path}"))
        }
        "move_path" => {
            let from = extract_string(parsed.as_ref(), &["from"])?;
            let to = extract_string(parsed.as_ref(), &["to"])?;
            Some(format!("move: {from} -> {to}"))
        }
        "remove_path" | "list_dir" => {
            extract_string(parsed.as_ref(), &["path"]).map(|path| format!("path: {path}"))
        }
        "search_text" => extract_string(parsed.as_ref(), &["query", "pattern"])
            .map(|query| format!("query: {query}")),
        "WebSearch" => {
            extract_string(parsed.as_ref(), &["query"]).map(|query| format!("query: {query}"))
        }
        "WebFetch" => extract_string(parsed.as_ref(), &["url"]).map(|url| format!("url: {url}")),
        "Task" | "Agent" => {
            extract_string(parsed.as_ref(), &["prompt"]).map(|prompt| format!("prompt: {prompt}"))
        }
        _ => extract_first_string(parsed.as_ref()).map(|value| format!("input: {value}")),
    }
}

fn extract_string<'a>(value: Option<&'a Value>, keys: &[&str]) -> Option<&'a str> {
    let object = value?.as_object()?;
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

fn extract_first_string(value: Option<&Value>) -> Option<&str> {
    value?
        .as_object()?
        .values()
        .find_map(Value::as_str)
}

fn output_preview(output: Option<&str>) -> Option<String> {
    let first_line = output?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    Some(truncate(first_line, 72))
}

fn truncate(text: &str, max_chars: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    let retained = chars.into_iter().take(max_chars.saturating_sub(1)).collect::<String>();
    format!("{retained}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapsed_tool_message_is_human_friendly() {
        let rendered = render_tool_message(
            "Tool WebSearch [ok]\ninput: {\"query\":\"rust tui streaming\"}\nRust TUI streaming guide",
            false,
        )
        .unwrap();
        let text = rendered
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Web Search [ok]"));
        assert!(text.contains("query: rust tui streaming"));
        assert!(text.contains("Rust TUI streaming guide"));
        assert!(!text.contains("input: {\"query\":\"rust tui streaming\"}"));
    }

    #[test]
    fn expanded_tool_message_falls_back_to_raw_view() {
        assert!(render_tool_message(
            "Tool WebSearch [ok]\ninput: {\"query\":\"rust tui streaming\"}\nRust TUI streaming guide",
            true,
        )
        .is_none());
    }
}
