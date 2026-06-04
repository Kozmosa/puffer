//! Tool name qualification for MCP servers.
//!
//! Mirrors codex's `qualify_tools` / `sanitize_responses_api_tool_name` so
//! tools advertised by MCP servers show up to the model as
//! `mcp__<server>__<tool>` — sanitized to match the OpenAI Responses API
//! function-name regex (`^[a-zA-Z0-9_-]+$`), capped at 64 bytes, with a
//! short hash suffix when the sanitized form would otherwise collide.

use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

pub const MCP_TOOL_NAME_DELIMITER: &str = "__";
pub const MAX_TOOL_NAME_LENGTH: usize = 64;
pub const CALLABLE_NAME_HASH_LEN: usize = 12;

/// One MCP tool listing paired with the qualified name we'll show the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedMcpTool {
    /// Server identifier as used in `runner.list_mcp_servers()`.
    pub server: String,
    /// Tool name as advertised by the MCP server.
    pub tool: String,
    /// `mcp__<server>__<tool>` after sanitization + collision handling.
    pub qualified_name: String,
}

/// Source row for `qualify_tools`. Kept separate from `QualifiedMcpTool`
/// so the caller can pass through pre-listed `(server, tool)` pairs
/// without committing to a specific schema type.
#[derive(Debug, Clone)]
pub struct McpToolKey {
    pub server: String,
    pub tool: String,
}

/// Builds qualified names for a flat list of MCP `(server, tool)` rows.
///
/// Names that would collide after sanitization get a short SHA-256 suffix.
/// Order is preserved.
pub fn qualify_tools(rows: Vec<McpToolKey>) -> Vec<QualifiedMcpTool> {
    let mut out: Vec<QualifiedMcpTool> = Vec::with_capacity(rows.len());
    let mut used: HashSet<String> = HashSet::new();
    let mut collision_counts: HashMap<String, usize> = HashMap::new();

    for row in rows {
        let base = build_qualified(&row.server, &row.tool, None);
        let qualified = if used.insert(base.clone()) {
            base
        } else {
            let collision_count = collision_counts.entry(base).or_insert(0);
            loop {
                *collision_count += 1;
                let candidate = build_qualified(
                    &row.server,
                    &row.tool,
                    Some(&disambiguator(&row, *collision_count)),
                );
                if used.insert(candidate.clone()) {
                    break candidate;
                }
            }
        };
        out.push(QualifiedMcpTool {
            server: row.server,
            tool: row.tool,
            qualified_name: qualified,
        });
    }
    out
}

/// Sanitizes a single name into the OpenAI Responses API function-name regex
/// `^[a-zA-Z0-9_-]+$`. Any other character is replaced with `_`.
pub fn sanitize_tool_name(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// Decodes a qualified name back into `(server, tool)`. Returns `None`
/// when the name is not in the expected `mcp__<server>__<tool>` form.
///
/// Note: when the original tool name contained the `__` delimiter the
/// roundtrip is ambiguous. The runtime executor stores the original
/// `(server, tool)` strings in `handler_args` rather than relying on this
/// helper for dispatch; this is provided only for diagnostics and tests.
pub fn split_qualified_name(qualified: &str) -> Option<(&str, &str)> {
    let stripped = qualified
        .strip_prefix("mcp")?
        .strip_prefix(MCP_TOOL_NAME_DELIMITER)?;
    let pos = stripped.find(MCP_TOOL_NAME_DELIMITER)?;
    Some((
        &stripped[..pos],
        &stripped[pos + MCP_TOOL_NAME_DELIMITER.len()..],
    ))
}

fn build_qualified(server: &str, tool: &str, suffix: Option<&str>) -> String {
    let server_clean = sanitize_tool_name(server);
    let tool_clean = sanitize_tool_name(tool);
    let suffix_part = suffix
        .map(|hash| format!("{}{}", MCP_TOOL_NAME_DELIMITER, hash))
        .unwrap_or_default();
    let fixed_len = "mcp".len()
        + MCP_TOOL_NAME_DELIMITER.len()
        + MCP_TOOL_NAME_DELIMITER.len()
        + suffix_part.len();
    let component_budget = MAX_TOOL_NAME_LENGTH.saturating_sub(fixed_len);
    let (server_budget, tool_budget) =
        component_budgets(server_clean.len(), tool_clean.len(), component_budget);
    let truncated_server = truncate_to_byte_budget(&server_clean, server_budget);
    let truncated_tool = truncate_to_byte_budget(&tool_clean, tool_budget);
    format!(
        "mcp{delim}{server}{delim}{tool}{suffix_part}",
        delim = MCP_TOOL_NAME_DELIMITER,
        server = truncated_server,
        tool = truncated_tool
    )
}

fn disambiguator(row: &McpToolKey, collision_index: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(row.server.as_bytes());
    hasher.update([0u8]);
    hasher.update(row.tool.as_bytes());
    if collision_index > 1 {
        hasher.update([0u8]);
        hasher.update(collision_index.to_string().as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(CALLABLE_NAME_HASH_LEN);
    for byte in digest.iter().take((CALLABLE_NAME_HASH_LEN + 1) / 2) {
        hex.push_str(&format!("{:02x}", byte));
    }
    hex.truncate(CALLABLE_NAME_HASH_LEN);
    hex
}

fn component_budgets(server_len: usize, tool_len: usize, max_bytes: usize) -> (usize, usize) {
    if server_len + tool_len <= max_bytes {
        return (server_len, tool_len);
    }
    if max_bytes == 0 {
        return (0, 0);
    }
    if max_bytes == 1 {
        return (1.min(server_len), 0);
    }

    let mut server_budget = server_len.min((max_bytes / 2).max(1));
    let mut tool_budget = tool_len.min(max_bytes - server_budget);
    if tool_budget == 0 && tool_len > 0 {
        tool_budget = 1;
        server_budget = server_budget.saturating_sub(1);
    }

    let mut spare = max_bytes - server_budget - tool_budget;
    if spare > 0 && server_budget < server_len {
        let add = spare.min(server_len - server_budget);
        server_budget += add;
        spare -= add;
    }
    if spare > 0 && tool_budget < tool_len {
        let add = spare.min(tool_len - tool_budget);
        tool_budget += add;
    }

    (server_budget, tool_budget)
}

fn truncate_to_byte_budget(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(server: &str, tool: &str) -> McpToolKey {
        McpToolKey {
            server: server.to_string(),
            tool: tool.to_string(),
        }
    }

    #[test]
    fn simple_qualification_uses_double_underscore_delimiter() {
        let q = qualify_tools(vec![key("playwright", "browser_navigate")]);
        assert_eq!(q[0].qualified_name, "mcp__playwright__browser_navigate");
    }

    #[test]
    fn invalid_characters_get_sanitized_to_underscore() {
        let q = qualify_tools(vec![key("my server", "do.thing")]);
        assert_eq!(q[0].qualified_name, "mcp__my_server__do_thing");
    }

    #[test]
    fn long_tool_name_truncated_within_64_byte_budget() {
        let long_tool = "a".repeat(200);
        let q = qualify_tools(vec![key("svr", &long_tool)]);
        assert!(q[0].qualified_name.len() <= MAX_TOOL_NAME_LENGTH);
        assert!(q[0].qualified_name.starts_with("mcp__svr__"));
    }

    #[test]
    fn collisions_get_sha_suffix() {
        let q = qualify_tools(vec![key("a", "x"), key("a", "x")]);
        assert_eq!(q[0].qualified_name, "mcp__a__x");
        assert_ne!(q[1].qualified_name, "mcp__a__x");
        assert!(q[1].qualified_name.starts_with("mcp__a__x__"));
        assert_eq!(
            q[1].qualified_name.len(),
            "mcp__a__x__".len() + CALLABLE_NAME_HASH_LEN
        );
    }

    #[test]
    fn long_server_name_truncated_within_64_byte_budget() {
        let long_server = "server".repeat(40);
        let q = qualify_tools(vec![key(&long_server, "navigate")]);

        assert!(q[0].qualified_name.len() <= MAX_TOOL_NAME_LENGTH);
        assert!(q[0].qualified_name.starts_with("mcp__"));
        assert!(q[0].qualified_name.ends_with("__navigate"));
    }

    #[test]
    fn long_server_and_tool_names_share_64_byte_budget() {
        let long_server = "s".repeat(120);
        let long_tool = "tool".repeat(80);
        let q = qualify_tools(vec![key(&long_server, &long_tool)]);
        let (server, tool) = split_qualified_name(&q[0].qualified_name).unwrap();

        assert!(q[0].qualified_name.len() <= MAX_TOOL_NAME_LENGTH);
        assert!(!server.is_empty());
        assert!(!tool.is_empty());
        assert!(server.len() < long_server.len());
        assert!(tool.len() < long_tool.len());
    }

    #[test]
    fn split_qualified_name_round_trips_simple_case() {
        let q = qualify_tools(vec![key("playwright", "navigate")]);
        let (s, t) = split_qualified_name(&q[0].qualified_name).unwrap();
        assert_eq!(s, "playwright");
        assert_eq!(t, "navigate");
    }

    #[test]
    fn empty_name_components_become_underscore() {
        assert_eq!(sanitize_tool_name(""), "_");
    }

    #[test]
    fn repeated_identical_collisions_still_get_unique_names() {
        let q = qualify_tools(vec![key("a", "x"), key("a", "x"), key("a", "x")]);
        let names = q
            .iter()
            .map(|row| row.qualified_name.as_str())
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(names.len(), q.len());
        assert_eq!(q[0].qualified_name, "mcp__a__x");
        assert!(q[1].qualified_name.starts_with("mcp__a__x__"));
        assert!(q[2].qualified_name.starts_with("mcp__a__x__"));
        assert_ne!(q[1].qualified_name, q[2].qualified_name);
    }

    #[test]
    fn collision_suffix_stays_within_truncated_name_budget() {
        let long_server = "server".repeat(40);
        let long_tool = "tool".repeat(80);
        let q = qualify_tools(vec![
            key(&long_server, &long_tool),
            key(&long_server, &long_tool),
        ]);
        let suffix = q[1]
            .qualified_name
            .rsplit(MCP_TOOL_NAME_DELIMITER)
            .next()
            .unwrap();

        assert!(q[0].qualified_name.len() <= MAX_TOOL_NAME_LENGTH);
        assert!(q[1].qualified_name.len() <= MAX_TOOL_NAME_LENGTH);
        assert_eq!(suffix.len(), CALLABLE_NAME_HASH_LEN);
    }
}
