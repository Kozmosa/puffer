use puffer_config::{ensure_workspace_dirs, ConfigPaths};
use puffer_session_store::SessionStore;
use puffer_test_support::{
    assert_normalized_snapshot, capture_tmux_visible_pane, start_tmux_command_with_size,
    temp_workspace, tmux_available, wait_for_tmux_text, TerminalSize,
};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const WIDE_TMUX_SIZE: TerminalSize = TerminalSize {
    rows: 36,
    cols: 120,
};
const MEDIUM_TMUX_SIZE: TerminalSize = TerminalSize {
    rows: 24,
    cols: 100,
};
const COMPACT_WIDE_TMUX_SIZE: TerminalSize = TerminalSize {
    rows: 22,
    cols: 84,
};
const NARROW_TMUX_SIZE: TerminalSize = TerminalSize { rows: 20, cols: 72 };

#[test]
fn tmux_home_wide_matches_snapshot() {
    assert_tmux_home_snapshot(WIDE_TMUX_SIZE, "tmux_home_wide_snapshot.txt");
}

#[test]
fn tmux_home_narrow_matches_snapshot() {
    assert_tmux_home_snapshot(NARROW_TMUX_SIZE, "tmux_home_narrow_snapshot.txt");
}

#[test]
fn tmux_home_compact_wide_matches_snapshot() {
    assert_tmux_home_snapshot(
        COMPACT_WIDE_TMUX_SIZE,
        "tmux_home_compact_wide_snapshot.txt",
    );
}

#[test]
fn tmux_home_medium_matches_snapshot() {
    assert_tmux_home_snapshot(MEDIUM_TMUX_SIZE, "tmux_home_medium_snapshot.txt");
}

#[test]
fn tmux_help_wide_matches_snapshot() {
    assert_tmux_help_snapshot(WIDE_TMUX_SIZE, "tmux_help_wide_snapshot.txt");
}

#[test]
fn tmux_help_medium_matches_snapshot() {
    assert_tmux_help_snapshot(MEDIUM_TMUX_SIZE, "tmux_help_medium_snapshot.txt");
}

#[test]
fn tmux_help_narrow_matches_snapshot() {
    assert_tmux_help_snapshot(NARROW_TMUX_SIZE, "tmux_help_narrow_snapshot.txt");
}

#[test]
fn tmux_help_compact_wide_matches_snapshot() {
    assert_tmux_help_snapshot(
        COMPACT_WIDE_TMUX_SIZE,
        "tmux_help_compact_wide_snapshot.txt",
    );
}

fn assert_tmux_home_snapshot(size: TerminalSize, snapshot_name: &str) {
    if !tmux_available() {
        return;
    }

    let (_tempdir, workspace) = configured_workspace();
    let session = start_tmux_with_home(&workspace, size);
    wait_for_tmux_text(&session, "Welcome to Puffer Code", Duration::from_secs(10)).unwrap();
    let capture = capture_tmux_visible_pane(&session).unwrap();
    assert_normalized_snapshot(
        &normalize_tmux_capture(&trim_blank_rows(&capture)),
        &snapshot_path(snapshot_name),
    )
    .unwrap();
}

fn assert_tmux_help_snapshot(size: TerminalSize, snapshot_name: &str) {
    if !tmux_available() {
        return;
    }

    let (_tempdir, workspace) = configured_workspace();
    let session = start_tmux_with_home_args(&workspace, size, &["/help"]);
    wait_for_tmux_text(&session, "Supported commands", Duration::from_secs(10)).unwrap();
    let capture = capture_tmux_visible_pane(&session).unwrap();
    assert_normalized_snapshot(
        &normalize_tmux_capture(&focused_help_capture(&capture, size)),
        &snapshot_path(snapshot_name),
    )
    .unwrap();
}

fn configured_workspace() -> (tempfile::TempDir, PathBuf) {
    let (tempdir, workspace) = temp_workspace().unwrap();
    let paths = ConfigPaths::discover(&workspace);
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    fs::create_dir_all(workspace.join("dockyard")).unwrap();
    let session = session_store
        .create_session(workspace.join("dockyard"))
        .unwrap();
    session_store
        .rename_session(session.id, "dockyard".to_string())
        .unwrap();
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    std::os::unix::fs::symlink(repo_root.join("resources"), workspace.join("resources")).unwrap();
    fs::write(
        workspace.join(".puffer/config.toml"),
        r#"
app_name = "Puffer Code"
default_provider = "anthropic"
default_model = "anthropic/claude-sonnet-4-5"
theme = "puffer"

[mascot]
id = "clawd"
display_name = "Clawd"
enabled = true

[ui]
no_alt_screen = true
tmux_golden_mode = true
"#,
    )
    .unwrap();
    fs::write(
        workspace.join(".puffer/auth.json"),
        r#"{
  "providers": {
    "anthropic": {
      "kind": "api_key",
      "key": "tmux-test-key"
    }
  }
}"#,
    )
    .unwrap();
    (tempdir, workspace)
}

fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
        .join(name)
}

fn start_tmux_with_home(
    workspace: &std::path::Path,
    size: TerminalSize,
) -> puffer_test_support::TmuxSession {
    start_tmux_with_home_args(workspace, size, &[])
}

fn start_tmux_with_home_args(
    workspace: &std::path::Path,
    size: TerminalSize,
    cli_args: &[&str],
) -> puffer_test_support::TmuxSession {
    let extra_args = cli_args
        .iter()
        .map(|value| shell_escape(value))
        .collect::<Vec<_>>()
        .join(" ");
    start_tmux_command_with_size(
        "sh",
        &[
            "-lc",
            &format!(
                "HOME='{}' '{}'{}{}",
                workspace.display(),
                env!("CARGO_BIN_EXE_puffer"),
                if extra_args.is_empty() { "" } else { " " },
                extra_args,
            ),
        ],
        Some(&workspace.join("dockyard")),
        size,
    )
    .unwrap()
}

fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn normalize_tmux_capture(capture: &str) -> String {
    capture
        .lines()
        .map(normalize_tmux_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn focused_tmux_capture(capture: &str, anchor: &str, before: usize, after: usize) -> String {
    let lines = capture.lines().collect::<Vec<_>>();
    let anchor_index = lines
        .iter()
        .position(|line| line.contains(anchor))
        .unwrap_or(0);
    let start = anchor_index.saturating_sub(before);
    let end = (anchor_index + after).min(lines.len());
    lines[start..end].join("\n")
}

fn focused_help_capture(capture: &str, size: TerminalSize) -> String {
    let after = if size.cols >= WIDE_TMUX_SIZE.cols { 12 } else { 10 };
    trim_common_padding(&focused_tmux_capture(capture, "Supported commands", 0, after))
}

fn trim_common_padding(capture: &str) -> String {
    let lines = capture.lines().collect::<Vec<_>>();
    let padding = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.chars().take_while(|ch| *ch == ' ').count())
        .min()
        .unwrap_or(0);
    lines.into_iter()
        .map(|line| line.chars().skip(padding).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn trim_blank_rows(capture: &str) -> String {
    let lines = capture.lines().collect::<Vec<_>>();
    let start = lines.iter().position(|line| !line.trim().is_empty()).unwrap_or(0);
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|index| index + 1)
        .unwrap_or(lines.len());
    lines[start..end].join("\n")
}

fn normalize_tmux_line(line: &str) -> String {
    let line = replace_session_marker(line);
    let line = replace_id_line(&line);
    replace_hex_runs(&line)
}

fn replace_session_marker(line: &str) -> String {
    for marker in ["session=session-", "│session-", "session-"] {
        if let Some(index) = line.find(marker) {
            let start = index + marker.len();
            let mut end = start;
            let bytes = line.as_bytes();
            while end < line.len() {
                let byte = bytes[end];
                if byte.is_ascii_alphanumeric() || byte == b'.' {
                    end += 1;
                } else {
                    break;
                }
            }
            let mut normalized = line.to_string();
            normalized.replace_range(start..end, "<id>");
            return normalized;
        }
    }
    line.to_string()
}

fn replace_id_line(line: &str) -> String {
    let Some(index) = line.find("Id: ") else {
        return line.to_string();
    };
    let start = index + 4;
    let mut end = start;
    let bytes = line.as_bytes();
    while end < line.len() {
        let byte = bytes[end];
        if byte.is_ascii_alphanumeric() {
            end += 1;
        } else {
            break;
        }
    }
    let mut normalized = line.to_string();
    normalized.replace_range(start..end, "<id>");
    normalized
}

fn replace_hex_runs(line: &str) -> String {
    let mut output = String::new();
    let mut hex_run = String::new();

    for ch in line.chars() {
        if ch.is_ascii_hexdigit() {
            hex_run.push(ch);
            continue;
        }

        flush_hex_run(&mut output, &mut hex_run);
        output.push(ch);
    }
    flush_hex_run(&mut output, &mut hex_run);
    output
}

fn flush_hex_run(output: &mut String, hex_run: &mut String) {
    if hex_run.len() >= 6 {
        output.push_str("<id>");
    } else {
        output.push_str(hex_run);
    }
    hex_run.clear();
}
