use reqwest::header::{HeaderMap, HeaderValue};

const DEFAULT_ORIGINATOR: &str = "codex_cli_rs";
const ORIGINATOR_OVERRIDE_ENV: &str = "CODEX_INTERNAL_ORIGINATOR_OVERRIDE";

pub(crate) fn default_originator() -> String {
    std::env::var(ORIGINATOR_OVERRIDE_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_ORIGINATOR.to_string())
}

pub(crate) fn codex_user_agent(version: &str, originator: &str) -> String {
    let os_info = os_info::get();
    let fallback = format!("{originator}/{version}");
    let candidate = format!(
        "{originator}/{version} ({} {}; {}) {}",
        os_info.os_type(),
        os_info.version(),
        os_info.architecture().unwrap_or("unknown"),
        terminal_user_agent_token(),
    );
    sanitize_user_agent(candidate, &fallback)
}

pub(crate) fn default_headers(version: &str, originator: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(originator) {
        headers.insert("originator", value);
    }
    if let Ok(value) = HeaderValue::from_str(&codex_user_agent(version, originator)) {
        headers.insert(reqwest::header::USER_AGENT, value);
    }
    headers
}

fn terminal_user_agent_token() -> String {
    if let Some(program) = env_non_empty("TERM_PROGRAM") {
        let version = env_non_empty("TERM_PROGRAM_VERSION");
        return sanitize_header_value(format_terminal_version(&program, version.as_deref()));
    }
    if let Some(version) = env_non_empty("WEZTERM_VERSION") {
        return sanitize_header_value(format!("wezterm/{version}"));
    }
    if env_exists("WT_SESSION") {
        return "windows-terminal".to_string();
    }
    if env_exists("KITTY_PID") {
        return "kitty".to_string();
    }
    if env_exists("ITERM_SESSION_ID") || env_exists("ITERM_PROFILE") {
        return "iterm".to_string();
    }
    if env_exists("GHOSTTY_RESOURCES_DIR") {
        return "ghostty".to_string();
    }
    if let Some(term) = env_non_empty("TERM") {
        return sanitize_header_value(term);
    }
    "unknown".to_string()
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_exists(name: &str) -> bool {
    std::env::var(name).is_ok()
}

fn format_terminal_version(name: &str, version: Option<&str>) -> String {
    match version.filter(|value| !value.is_empty()) {
        Some(version) => format!("{name}/{version}"),
        None => name.to_string(),
    }
}

fn sanitize_header_value(value: String) -> String {
    value.replace(
        |character: char| !matches!(character, '0'..='9' | 'A'..='Z' | 'a'..='z' | '-' | '_' | '.' | '/'),
        "_",
    )
}

fn sanitize_user_agent(candidate: String, fallback: &str) -> String {
    if HeaderValue::from_str(&candidate).is_ok() {
        return candidate;
    }

    let sanitized = candidate
        .chars()
        .map(|character| {
            if matches!(character, ' '..='~') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if !sanitized.is_empty() && HeaderValue::from_str(&sanitized).is_ok() {
        sanitized
    } else if HeaderValue::from_str(fallback).is_ok() {
        fallback.to_string()
    } else {
        DEFAULT_ORIGINATOR.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{codex_user_agent, default_headers, default_originator};

    #[test]
    fn default_originator_matches_codex() {
        assert_eq!(default_originator(), "codex_cli_rs");
    }

    #[test]
    fn codex_user_agent_starts_with_originator_and_version() {
        let user_agent = codex_user_agent("0.1.0", "codex_cli_rs");
        assert!(user_agent.starts_with("codex_cli_rs/0.1.0 "));
    }

    #[test]
    fn default_headers_include_originator_and_user_agent() {
        let headers = default_headers("0.1.0", "codex_cli_rs");
        assert_eq!(
            headers.get("originator").and_then(|value| value.to_str().ok()),
            Some("codex_cli_rs")
        );
        assert!(headers.contains_key(reqwest::header::USER_AGENT));
    }
}
