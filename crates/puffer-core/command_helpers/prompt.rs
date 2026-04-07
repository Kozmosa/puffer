use super::append_tool_invocations;
use super::common::open_text_file_in_editor;
use super::emit_system;
use crate::{AppState, MessageRole};
use anyhow::Result;
use puffer_config::{ensure_workspace_dirs, ConfigPaths};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;
use puffer_session_store::{SessionStore, TranscriptEvent};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

/// Describes how a prompt command should be handled after specialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PromptCommandPreparation {
    /// The command already produced local output and should skip provider execution.
    HandledLocally,
    /// The command should execute through the provider with a custom prompt body.
    PromptOverride(String),
}

/// Returns any specialized handling required for prompt commands with local semantics.
pub(crate) fn prepare_prompt_command_specialization(
    state: &mut AppState,
    session_store: &SessionStore,
    command_name: &str,
    args: &str,
) -> Result<Option<PromptCommandPreparation>> {
    match command_name {
        "compact" => Ok(Some(prepare_compact_prompt_command(
            state,
            session_store,
            args,
        )?)),
        "plan" => Ok(Some(prepare_plan_prompt_command(
            state,
            session_store,
            args,
        )?)),
        "pr-comments" => Ok(Some(prepare_pr_comments_prompt_command(args))),
        _ => Ok(None),
    }
}

/// Prepares `/compact` by generating a provider-driven compaction prompt override.
pub(crate) fn prepare_compact_prompt_command(
    state: &mut AppState,
    session_store: &SessionStore,
    args: &str,
) -> Result<PromptCommandPreparation> {
    if state.transcript.is_empty() {
        emit_system(
            state,
            session_store,
            "No messages are available to compact.".to_string(),
        )?;
        return Ok(PromptCommandPreparation::HandledLocally);
    }
    Ok(PromptCommandPreparation::PromptOverride(
        build_compact_prompt_override(state, args),
    ))
}

/// Handles `/plan` local behaviors and generates prompt overrides for plan creation.
pub(crate) fn prepare_plan_prompt_command(
    state: &mut AppState,
    session_store: &SessionStore,
    args: &str,
) -> Result<PromptCommandPreparation> {
    let plan_path = ensure_plan_file(state)?;
    let trimmed = args.trim();

    if trimmed.is_empty() && !state.plan_mode {
        state.plan_mode = true;
        emit_system(
            state,
            session_store,
            format!(
                "Enabled plan mode.\nPlan file: {}\nUse `/plan <description>` to draft a plan or `/plan open` to edit it.",
                plan_path.display()
            ),
        )?;
        return Ok(PromptCommandPreparation::HandledLocally);
    }

    if trimmed.is_empty() || trimmed == "show" {
        state.plan_mode = true;
        let plan_body = fs::read_to_string(&plan_path).unwrap_or_default();
        emit_system(
            state,
            session_store,
            format!(
                "Current plan file: {}\n{}",
                plan_path.display(),
                plan_body.trim_end()
            ),
        )?;
        return Ok(PromptCommandPreparation::HandledLocally);
    }

    if trimmed == "open" {
        state.plan_mode = true;
        let status = match open_text_file_in_editor(&plan_path) {
            Ok(status) => status,
            Err(error) => format!(
                "Could not open the plan in an editor: {error}\nPath: {}",
                plan_path.display()
            ),
        };
        emit_system(state, session_store, status)?;
        return Ok(PromptCommandPreparation::HandledLocally);
    }

    state.plan_mode = true;
    let current_plan = fs::read_to_string(&plan_path).unwrap_or_default();
    let prompt = build_plan_prompt_override(trimmed, &plan_path, &current_plan);
    Ok(PromptCommandPreparation::PromptOverride(prompt))
}

/// Builds a stronger `/pr-comments` prompt that follows Claude-style CLI guidance.
pub(crate) fn prepare_pr_comments_prompt_command(args: &str) -> PromptCommandPreparation {
    PromptCommandPreparation::PromptOverride(build_pr_comments_prompt_override(args))
}

/// Executes the provider-backed `/compact` prompt and persists the compacted transcript.
pub(crate) fn execute_compact_prompt_command(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &mut AuthStore,
    session_store: &SessionStore,
    rendered: &str,
) -> Result<()> {
    record_specialized_prompt_request(state, session_store, rendered)?;
    match crate::runtime::execute_user_prompt(state, resources, providers, auth_store, rendered) {
        Ok(turn) => {
            append_tool_invocations(state, session_store, &turn.tool_invocations)?;
            finalize_compact_prompt_command(state, session_store, &turn.assistant_text)
        }
        Err(error) => emit_system(
            state,
            session_store,
            format!("Prompt command /compact failed: {error}"),
        ),
    }
}

/// Executes the provider-backed `/plan` prompt and persists the resulting plan file.
pub(crate) fn execute_plan_prompt_command(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &mut AuthStore,
    session_store: &SessionStore,
    rendered: &str,
) -> Result<()> {
    record_specialized_prompt_request(state, session_store, rendered)?;
    match crate::runtime::execute_user_prompt(state, resources, providers, auth_store, rendered) {
        Ok(turn) => {
            append_tool_invocations(state, session_store, &turn.tool_invocations)?;
            finalize_plan_prompt_command(state, session_store, &turn.assistant_text)
        }
        Err(error) => emit_system(
            state,
            session_store,
            format!("Prompt command /plan failed: {error}"),
        ),
    }
}

fn record_specialized_prompt_request(
    state: &mut AppState,
    session_store: &SessionStore,
    rendered: &str,
) -> Result<()> {
    state.push_message(MessageRole::User, rendered.to_string());
    session_store.append_event(
        state.session.id,
        TranscriptEvent::UserMessage {
            text: rendered.to_string(),
        },
    )?;
    Ok(())
}

/// Applies a provider-generated compaction summary and persists the transcript rewrite.
pub(crate) fn finalize_compact_prompt_command(
    state: &mut AppState,
    session_store: &SessionStore,
    summary: &str,
) -> Result<()> {
    session_store.append_transcript_clear(state.session.id)?;
    state.apply_transcript_rewrite(&puffer_session_store::TranscriptRewrite::Clear);
    emit_system(
        state,
        session_store,
        format!("Compacted conversation summary:\n{}", summary.trim_end()),
    )
}

/// Persists provider-generated plan text to disk and emits a short status message.
pub(crate) fn finalize_plan_prompt_command(
    state: &mut AppState,
    session_store: &SessionStore,
    plan_text: &str,
) -> Result<()> {
    let plan_path = persist_plan_output(state, plan_text)?;
    emit_system(
        state,
        session_store,
        format!(
            "Updated plan file: {}\n\n{}",
            plan_path.display(),
            plan_text.trim_end()
        ),
    )
}

/// Persists provider-generated plan text to the session plan file.
pub(crate) fn persist_plan_output(state: &AppState, plan_text: &str) -> Result<PathBuf> {
    let plan_path = ensure_plan_file(state)?;
    fs::write(&plan_path, plan_text)?;
    Ok(plan_path)
}

/// Renders the active plan-mode context block for provider requests.
pub(crate) fn plan_mode_context_message(state: &AppState) -> Result<Option<String>> {
    if !state.plan_mode {
        return Ok(None);
    }
    let plan_path = ensure_plan_file(state)?;
    let plan_text = fs::read_to_string(&plan_path).unwrap_or_default();
    let plan_body = if plan_text.trim().is_empty() {
        "<empty>"
    } else {
        plan_text.trim_end()
    };
    Ok(Some(format!(
        "Plan mode is active. Focus on exploration, analysis, and refining the plan instead of implementing code.\n\
The active plan file is: {}\n\
\n\
Current plan contents:\n{}",
        plan_path.display(),
        plan_body
    )))
}

fn build_compact_prompt_override(state: &AppState, args: &str) -> String {
    let trimmed_instruction = args.trim();
    let mut user_messages = 0usize;
    let mut assistant_messages = 0usize;
    let mut system_messages = 0usize;
    let mut highlights = Vec::new();

    for message in state.transcript.iter().rev() {
        match message.role {
            MessageRole::User => user_messages += 1,
            MessageRole::Assistant => assistant_messages += 1,
            MessageRole::System => system_messages += 1,
        }
        if highlights.len() >= 8 {
            continue;
        }
        let compact_line = single_line_excerpt(&message.text);
        if compact_line.is_empty() {
            continue;
        }
        let role = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        highlights.push(format!("- {role}: {compact_line}"));
    }
    highlights.reverse();

    let mut text = String::from(
        "Summarize the current conversation so work can continue with a compact preserved context.\n",
    );
    let _ = writeln!(
        &mut text,
        "messages: user={} assistant={} system={}",
        user_messages, assistant_messages, system_messages
    );
    if !trimmed_instruction.is_empty() {
        let _ = writeln!(&mut text, "custom_instruction: {trimmed_instruction}");
    }
    text.push_str("Return only the compact summary that should remain in context.\n");
    if highlights.is_empty() {
        text.push_str("highlights:\n- <no non-empty messages>");
    } else {
        text.push_str("highlights:\n");
        text.push_str(&highlights.join("\n"));
    }
    text
}

fn build_plan_prompt_override(focus: &str, plan_path: &PathBuf, current_plan: &str) -> String {
    format!(
        "You are updating the session plan file.\n\
Plan file: {}\n\
Requested focus: {}\n\
\n\
Return an updated, concrete implementation plan with numbered steps.\n\
Keep it scoped to the requested focus and include verification steps.\n\
\n\
Current plan:\n{}",
        plan_path.display(),
        focus,
        if current_plan.trim().is_empty() {
            "<empty>"
        } else {
            current_plan.trim_end()
        }
    )
}

fn build_pr_comments_prompt_override(args: &str) -> String {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return String::from(
            "Collect and summarize GitHub pull-request comments relevant to this workspace.\n\
1. Run `gh pr list --limit 20` to find candidate PRs.\n\
2. Pick the most relevant PR and run `gh pr view <number> --comments`.\n\
3. Group comments by theme and identify unresolved or blocking feedback.\n\
4. Provide a concise action plan for addressing outstanding comments.\n\
\n\
If no PR is available, explain what failed and what command output you saw.",
        );
    }

    format!(
        "Collect and summarize GitHub pull-request comments.\n\
Provided target/context: {}\n\
\n\
1. If a PR number is present, run `gh pr view <number> --comments`.\n\
2. Otherwise run `gh pr list --limit 20` and select the best match before viewing comments.\n\
3. Summarize comments by reviewer and status (open vs resolved).\n\
4. Produce a concrete response plan with next edits/tests.",
        trimmed
    )
}

fn single_line_excerpt(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or("");
    if line.chars().count() <= 120 {
        line.to_string()
    } else {
        let mut shortened = String::new();
        for ch in line.chars().take(117) {
            shortened.push(ch);
        }
        shortened.push_str("...");
        shortened
    }
}

fn ensure_plan_file(state: &AppState) -> Result<PathBuf> {
    let paths = ConfigPaths::discover(&state.cwd);
    ensure_workspace_dirs(&paths)?;
    let plan_dir = paths.workspace_config_dir.join("plans");
    fs::create_dir_all(&plan_dir)?;
    let plan_path = plan_dir.join(format!("{}.md", state.session.id));
    if !plan_path.exists() {
        fs::write(&plan_path, default_plan_file_contents())?;
    }
    Ok(plan_path)
}

fn default_plan_file_contents() -> &'static str {
    "# Current Plan\n\n- Add concrete implementation steps here.\n"
}

#[cfg(test)]
mod tests {
    use super::{
        prepare_compact_prompt_command, prepare_plan_prompt_command,
        prepare_pr_comments_prompt_command, prepare_prompt_command_specialization,
        PromptCommandPreparation,
    };
    use crate::{AppState, MessageRole};
    use puffer_config::{ensure_workspace_dirs, ConfigPaths, PufferConfig};
    use puffer_session_store::SessionStore;
    use tempfile::tempdir;
    use tempfile::TempDir;

    #[test]
    fn compact_specialization_returns_prompt_override() {
        let fixture = sample_state();
        let mut state = fixture.state;
        let session_store = fixture.session_store;
        state.push_message(MessageRole::User, "Ship this change.");
        state.push_message(MessageRole::Assistant, "Implemented and tested.");

        let outcome =
            prepare_compact_prompt_command(&mut state, &session_store, "focus on tests").unwrap();

        match outcome {
            PromptCommandPreparation::PromptOverride(prompt) => {
                assert!(prompt.contains("Summarize the current conversation"));
                assert!(prompt.contains("custom_instruction: focus on tests"));
            }
            PromptCommandPreparation::HandledLocally => {
                panic!("expected compact prompt override")
            }
        }
    }

    #[test]
    fn plan_specialization_enables_mode_and_supports_show_and_open() {
        let fixture = sample_state();
        let mut state = fixture.state;
        let session_store = fixture.session_store;

        let show_outcome = prepare_plan_prompt_command(&mut state, &session_store, "").unwrap();
        assert_eq!(show_outcome, PromptCommandPreparation::HandledLocally);
        assert!(state.plan_mode);
        assert!(state
            .transcript
            .last()
            .unwrap()
            .text
            .contains("Enabled plan mode."));

        let open_outcome = prepare_plan_prompt_command(&mut state, &session_store, "open").unwrap();
        assert_eq!(open_outcome, PromptCommandPreparation::HandledLocally);
        assert!(!state.transcript.last().unwrap().text.is_empty());
    }

    #[test]
    fn plan_specialization_with_description_overrides_prompt() {
        let fixture = sample_state();
        let mut state = fixture.state;
        let session_store = fixture.session_store;
        let outcome = prepare_plan_prompt_command(
            &mut state,
            &session_store,
            "stabilize slash-command parity",
        )
        .unwrap();

        match outcome {
            PromptCommandPreparation::PromptOverride(prompt) => {
                assert!(prompt.contains("Requested focus: stabilize slash-command parity"));
                assert!(prompt.contains("Current plan:"));
                assert!(prompt.contains(".puffer/plans/"));
            }
            PromptCommandPreparation::HandledLocally => {
                panic!("expected prompt override for non-empty plan arguments")
            }
        }
    }

    #[test]
    fn pr_comments_specialization_builds_cli_focused_prompt() {
        let empty = prepare_pr_comments_prompt_command("");
        let targeted = prepare_pr_comments_prompt_command("123");

        match empty {
            PromptCommandPreparation::PromptOverride(prompt) => {
                assert!(prompt.contains("gh pr list --limit 20"));
                assert!(prompt.contains("gh pr view <number> --comments"));
            }
            PromptCommandPreparation::HandledLocally => panic!("expected prompt override"),
        }
        match targeted {
            PromptCommandPreparation::PromptOverride(prompt) => {
                assert!(prompt.contains("Provided target/context: 123"));
                assert!(prompt.contains("gh pr view <number> --comments"));
            }
            PromptCommandPreparation::HandledLocally => panic!("expected prompt override"),
        }
    }

    #[test]
    fn dispatcher_helper_routes_known_prompt_specializations() {
        let fixture = sample_state();
        let mut state = fixture.state;
        let session_store = fixture.session_store;
        state.push_message(MessageRole::User, "summarize this");
        let compact =
            prepare_prompt_command_specialization(&mut state, &session_store, "compact", "")
                .unwrap();
        match compact {
            Some(PromptCommandPreparation::PromptOverride(prompt)) => {
                assert!(prompt.contains("Summarize the current conversation"));
            }
            _ => panic!("expected compact prompt override"),
        }

        let pr_comments =
            prepare_prompt_command_specialization(&mut state, &session_store, "pr-comments", "")
                .unwrap();
        match pr_comments {
            Some(PromptCommandPreparation::PromptOverride(prompt)) => {
                assert!(prompt.contains("Collect and summarize GitHub pull-request comments"));
            }
            _ => panic!("expected pr-comments prompt override"),
        }

        let none = prepare_prompt_command_specialization(&mut state, &session_store, "review", "")
            .unwrap();
        assert!(none.is_none());
    }

    struct TestFixture {
        #[allow(dead_code)]
        tempdir: TempDir,
        state: AppState,
        session_store: SessionStore,
    }

    fn sample_state() -> TestFixture {
        let tempdir = tempdir().unwrap();
        let paths = ConfigPaths::discover(tempdir.path());
        ensure_workspace_dirs(&paths).unwrap();
        let session_store = SessionStore::from_paths(&paths).unwrap();
        let session = session_store
            .create_session(tempdir.path().to_path_buf())
            .unwrap();
        let state = AppState::new(
            PufferConfig::default(),
            tempdir.path().to_path_buf(),
            session,
        );
        TestFixture {
            tempdir,
            state,
            session_store,
        }
    }
}
