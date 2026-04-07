use crate::{AppState, MessageRole};
use anyhow::{Context, Result};
use arboard::Clipboard;
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::{skill_by_name, LoadedResources};
use puffer_session_store::{GitDiffSnapshot, SessionStore, TranscriptEvent};
use puffer_tools::ToolRegistry;
use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Lists loaded skills in slash-command form.
pub(crate) fn list_skills(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    if resources.skills.is_empty() {
        return emit_system(state, session_store, "No skills are available.".to_string());
    }
    let mut text = String::from("Available skills:\n");
    for skill in &resources.skills {
        let _ = writeln!(
            &mut text,
            "/skill:{} - {}",
            skill.value.name, skill.value.description
        );
    }
    emit_system(state, session_store, text)
}

/// Summarizes workspace health, loaded resources, and auth state.
pub(crate) fn run_doctor(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    session_store: &SessionStore,
) -> Result<()> {
    let registry = ToolRegistry::from_resources(resources);
    let mut text = String::from("Puffer doctor summary:\n");
    let _ = writeln!(
        &mut text,
        "provider={} model={}",
        state.current_provider.as_deref().unwrap_or("<unset>"),
        state.current_model.as_deref().unwrap_or("<unset>")
    );
    let _ = writeln!(&mut text, "tool_count={}", registry.tools().count());
    let _ = writeln!(
        &mut text,
        "provider_count={}",
        providers.providers().count()
    );
    let discovery_count = providers
        .provider_entries()
        .filter(|provider| provider.descriptor.discovery.is_some())
        .count();
    let _ = writeln!(&mut text, "providers_with_discovery={discovery_count}");
    let _ = writeln!(
        &mut text,
        "stored_auth_providers={}",
        auth_store.provider_ids().count()
    );
    let _ = writeln!(&mut text, "hooks={}", resources.hooks.len());
    let _ = writeln!(
        &mut text,
        "resource_diagnostics={}",
        resources.diagnostics.len()
    );
    let _ = writeln!(&mut text, "recorded_tasks={}", state.tasks().len());
    let _ = writeln!(&mut text, "working_dirs={}", state.working_dirs.len());
    let _ = writeln!(&mut text, "transcript_messages={}", state.transcript.len());
    if !resources.diagnostics.is_empty() {
        let _ = writeln!(&mut text, "Diagnostics:");
        for diagnostic in &resources.diagnostics {
            let _ = writeln!(&mut text, "- {diagnostic}");
        }
    }
    emit_system(state, session_store, text)
}

/// Copies the latest assistant message or echoes it when clipboard access fails.
pub(crate) fn copy_last_message(state: &mut AppState, session_store: &SessionStore) -> Result<()> {
    let last = state
        .transcript
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::Assistant)
        .map(|message| message.text.clone())
        .unwrap_or_default();
    if last.is_empty() {
        return emit_system(
            state,
            session_store,
            "No assistant response is available to copy.".to_string(),
        );
    }

    match Clipboard::new().and_then(|mut clipboard| clipboard.set_text(last.clone())) {
        Ok(()) => emit_system(
            state,
            session_store,
            "Copied the latest assistant response.".to_string(),
        ),
        Err(_) => emit_system(
            state,
            session_store,
            format!("Latest assistant response:\n{last}"),
        ),
    }
}

/// Prints a compact summary of transcript and loaded-resource context.
pub(crate) fn describe_context(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    emit_system(
        state,
        session_store,
        format!(
            "Context summary:\ntranscript_messages={}\nworking_dirs={}\nprompts={}\nskills={}\nplugins={}",
            state.transcript.len(),
            state.working_dirs.len(),
            resources.prompts.len(),
            resources.skills.len(),
            resources.plugins.len()
        ),
    )
}

/// Shows the current git status summary for the workspace.
pub(crate) fn describe_git_diff(state: &mut AppState, session_store: &SessionStore) -> Result<()> {
    emit_system(
        state,
        session_store,
        render_git_diff_summary(&state.cwd, session_store, state.session.id),
    )
}

/// Opens or creates a text file in an external editor and returns a status line.
pub(crate) fn open_text_file_in_editor(path: &Path) -> Result<String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if !path.exists() {
        std::fs::write(path, b"")
            .with_context(|| format!("failed to create {}", path.display()))?;
    }

    if let Some((source, command)) = configured_editor() {
        run_shell_editor(&command, path)
            .with_context(|| format!("failed to launch editor configured by {source}"))?;
        return Ok(format!("Opened {} with {source}.", path.display()));
    }

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!("No non-interactive editor is configured. Set $VISUAL or $EDITOR.");
    }

    for fallback in ["nano", "vim", "vi"] {
        match Command::new(fallback).arg(path).status() {
            Ok(status) if status.success() => {
                return Ok(format!("Opened {} with {}.", path.display(), fallback));
            }
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("failed to launch fallback editor"),
        }
    }

    anyhow::bail!("No editor is configured. Set $VISUAL or $EDITOR.");
}

/// Renders a UTF-8 QR block when `qrencode` is available on PATH.
pub(crate) fn render_utf8_qr(data: &str) -> Option<String> {
    let mut child = Command::new("qrencode")
        .args(["-t", "UTF8", "-m", "1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(data.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let qr = String::from_utf8(output.stdout).ok()?;
    let trimmed = qr.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Expands a `/skill:<name>` command into the loaded skill contents.
pub(crate) fn execute_skill_command(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    skill_name: &str,
) -> Result<()> {
    if let Some(skill) = skill_by_name(resources, skill_name) {
        emit_system(
            state,
            session_store,
            format!(
                "Skill {}\n{}\n\n{}",
                skill.value.name, skill.value.description, skill.value.content
            ),
        )
    } else {
        emit_system(state, session_store, format!("Unknown skill {skill_name}."))
    }
}

/// Appends a system message to the in-memory transcript and session log.
pub(crate) fn emit_system(
    state: &mut AppState,
    session_store: &SessionStore,
    text: String,
) -> Result<()> {
    state.push_message(MessageRole::System, text.clone());
    session_store.append_event(state.session.id, TranscriptEvent::SystemMessage { text })?;
    Ok(())
}

/// Rewinds the transcript to the requested point.
pub(crate) fn rewind_transcript(
    state: &mut AppState,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    if state.transcript.is_empty() {
        return emit_system(
            state,
            session_store,
            "Transcript is already empty.".to_string(),
        );
    }
    let trimmed = args.trim();
    let Some(pop_count) = rewind_pop_count(state, trimmed) else {
        return emit_system(
            state,
            session_store,
            if trimmed.is_empty() {
                "Nothing is available to rewind to yet.".to_string()
            } else {
                format!("Unknown rewind target `{trimmed}`.")
            },
        );
    };
    if pop_count == 0 {
        return emit_system(
            state,
            session_store,
            "Already at the selected rewind point.".to_string(),
        );
    }
    session_store.append_transcript_pop_last(state.session.id, pop_count)?;
    state.apply_transcript_rewrite(&puffer_session_store::TranscriptRewrite::PopLast {
        count: pop_count,
    });
    let message = if trimmed.is_empty() {
        "Removed the latest rendered transcript item.".to_string()
    } else {
        format!("Rewound conversation to before user turn {trimmed}.")
    };
    emit_system(state, session_store, message)
}

/// Records post-command session state and a git diff history snapshot.
pub(crate) fn record_command_checkpoint(
    state: &AppState,
    session_store: &SessionStore,
    command_name: &str,
    args: &str,
) -> Result<()> {
    session_store.append_event(state.session.id, state.snapshot_event())?;
    let snapshot = capture_git_diff_snapshot(&state.cwd, command_name, args);
    session_store.append_git_diff_snapshot(state.session.id, snapshot)
}

/// Returns terminal setup guidance for the current runtime mode.
pub(crate) fn terminal_setup_advice(state: &AppState) -> String {
    format!(
        "Terminal setup:\n- current cwd: {}\n- no_alt_screen: {}\n- tmux_golden_mode: {}",
        state.cwd.display(),
        state.config.ui.no_alt_screen,
        state.config.ui.tmux_golden_mode
    )
}

fn render_git_diff_summary(
    cwd: &PathBuf,
    session_store: &SessionStore,
    session_id: uuid::Uuid,
) -> String {
    let current = render_current_git_diff_summary(cwd);
    let history = render_git_diff_history(session_store, session_id);
    if history.is_empty() {
        current
    } else {
        format!("{current}\n\nRecent turn-by-turn diff snapshots:\n{history}")
    }
}

fn render_current_git_diff_summary(cwd: &PathBuf) -> String {
    let status = match run_git(cwd, &["status", "--short"]) {
        Ok(status) => status,
        Err(error) => return error,
    };
    if status.trim().is_empty() {
        return "Working tree is clean.".to_string();
    }

    let mut text = String::new();
    let _ = writeln!(&mut text, "Git status:");
    let _ = writeln!(&mut text, "{}", status.trim_end());

    append_stat_section(&mut text, cwd, "Unstaged diffstat", &["diff", "--stat"]);
    append_stat_section(
        &mut text,
        cwd,
        "Staged diffstat",
        &["diff", "--cached", "--stat"],
    );
    append_patch_section(&mut text, cwd, "Unstaged patch", &["diff"]);
    append_patch_section(&mut text, cwd, "Staged patch", &["diff", "--cached"]);

    text.trim_end().to_string()
}

fn render_git_diff_history(session_store: &SessionStore, session_id: uuid::Uuid) -> String {
    let Ok(record) = session_store.load_session(session_id) else {
        return String::new();
    };
    let snapshots = record
        .events
        .into_iter()
        .filter_map(|event| match event {
            TranscriptEvent::GitDiffSnapshot { snapshot } => Some(snapshot),
            _ => None,
        })
        .collect::<Vec<_>>();
    if snapshots.is_empty() {
        return String::new();
    }

    snapshots
        .into_iter()
        .rev()
        .take(6)
        .enumerate()
        .map(|(index, snapshot)| render_git_snapshot(index + 1, &snapshot))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_git_snapshot(number: usize, snapshot: &GitDiffSnapshot) -> String {
    let mut text = format!("{}. {}", number, snapshot.command);
    if !snapshot.status.trim().is_empty() {
        let _ = write!(&mut text, "\nstatus:\n{}", snapshot.status.trim_end());
    }
    if !snapshot.unstaged_diffstat.trim().is_empty() {
        let _ = write!(
            &mut text,
            "\nunstaged_diffstat:\n{}",
            snapshot.unstaged_diffstat.trim_end()
        );
    }
    if !snapshot.staged_diffstat.trim().is_empty() {
        let _ = write!(
            &mut text,
            "\nstaged_diffstat:\n{}",
            snapshot.staged_diffstat.trim_end()
        );
    }
    if !snapshot.patch_excerpt.trim().is_empty() {
        let _ = write!(
            &mut text,
            "\npatch_excerpt:\n{}",
            snapshot.patch_excerpt.trim_end()
        );
    }
    text
}

fn capture_git_diff_snapshot(cwd: &PathBuf, command_name: &str, args: &str) -> GitDiffSnapshot {
    let command = if args.is_empty() {
        format!("/{command_name}")
    } else {
        format!("/{command_name} {args}")
    };
    let status = run_git(cwd, &["status", "--short"]).unwrap_or_else(|error| error);
    let unstaged_diffstat = run_git(cwd, &["diff", "--stat"]).unwrap_or_else(|error| error);
    let staged_diffstat =
        run_git(cwd, &["diff", "--cached", "--stat"]).unwrap_or_else(|error| error);
    let patch = run_git(cwd, &["diff", "--cached", "--patch", "--no-ext-diff"])
        .or_else(|_| run_git(cwd, &["diff", "--patch", "--no-ext-diff"]))
        .unwrap_or_default();
    let (patch_excerpt, truncated) = truncate_lines(&patch, 120);
    let patch_excerpt = if truncated {
        format!("{}\n... output truncated ...", patch_excerpt.trim_end())
    } else {
        patch_excerpt
    };
    GitDiffSnapshot {
        command,
        status,
        unstaged_diffstat,
        staged_diffstat,
        patch_excerpt,
    }
}

fn rewind_pop_count(state: &AppState, trimmed: &str) -> Option<usize> {
    if trimmed.is_empty() {
        return Some(1);
    }
    let selected_turn = trimmed.parse::<usize>().ok()?;
    if selected_turn == 0 {
        return None;
    }
    let transcript_index = state
        .transcript
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == MessageRole::User)
        .nth(selected_turn.saturating_sub(1))
        .map(|(index, _)| index)?;
    Some(state.transcript.len().saturating_sub(transcript_index))
}

fn append_stat_section(text: &mut String, cwd: &PathBuf, title: &str, args: &[&str]) {
    match run_git(cwd, args) {
        Ok(content) if !content.trim().is_empty() => {
            let _ = writeln!(text);
            let _ = writeln!(text, "{title}:");
            let _ = writeln!(text, "{}", content.trim_end());
        }
        Ok(_) => {}
        Err(error) => {
            let _ = writeln!(text);
            let _ = writeln!(text, "{title}:");
            let _ = writeln!(text, "{error}");
        }
    }
}

fn append_patch_section(text: &mut String, cwd: &PathBuf, title: &str, args: &[&str]) {
    match run_git(cwd, args) {
        Ok(content) if !content.trim().is_empty() => {
            let (truncated, was_truncated) = truncate_lines(&content, 220);
            let _ = writeln!(text);
            let _ = writeln!(text, "{title}:");
            let _ = writeln!(text, "{}", truncated.trim_end());
            if was_truncated {
                let _ = writeln!(text, "... output truncated ...");
            }
        }
        Ok(_) => {}
        Err(error) => {
            let _ = writeln!(text);
            let _ = writeln!(text, "{title}:");
            let _ = writeln!(text, "{error}");
        }
    }
}

fn truncate_lines(content: &str, max_lines: usize) -> (String, bool) {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.len() <= max_lines {
        return (content.to_string(), false);
    }
    let truncated = lines
        .into_iter()
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n");
    (truncated, true)
}

fn run_git(cwd: &PathBuf, args: &[&str]) -> std::result::Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|error| format!("Failed to run git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "Failed to run git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn configured_editor() -> Option<(&'static str, String)> {
    std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| ("$VISUAL", value))
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|value| ("$EDITOR", value))
        })
}

fn run_shell_editor(command: &str, path: &Path) -> Result<()> {
    let status = Command::new("sh")
        .arg("-lc")
        .arg(format!("{command} \"$PUFFER_EDITOR_TARGET\""))
        .env("PUFFER_EDITOR_TARGET", path)
        .status()
        .context("failed to run shell for editor")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("editor command exited with {}", status);
    }
}
