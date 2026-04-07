use super::emit_system;
use crate::AppState;
use anyhow::Result;
use puffer_session_store::{SessionStore, SessionSummary};
use std::fmt::Write as _;

/// Handles `/resume` by listing resumable sessions or switching to a unique match.
pub(crate) fn handle_resume_command(
    state: &mut AppState,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    let sessions = resumable_sessions(session_store, state.session.id)?;
    let query = args.trim();
    if query.is_empty() {
        return emit_system(state, session_store, render_resume_listing(&sessions));
    }

    let matches = search_sessions(&sessions, query);
    let Some(best_match) = matches.first() else {
        return emit_system(
            state,
            session_store,
            format!("No session matched `{query}`.\nRun /resume to pick from recent sessions."),
        );
    };

    let best_rank = best_match.rank;
    let best_rank_count = matches
        .iter()
        .take_while(|candidate| candidate.rank == best_rank)
        .count();
    if matches.len() == 1 || (best_rank_count == 1 && best_rank.is_precise()) {
        return resume_into_session(state, session_store, &best_match.session);
    }

    emit_system(
        state,
        session_store,
        render_resume_ambiguity(query, &matches),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ResumeMatchRank {
    ExactId,
    PrefixId,
    ExactName,
    ExactSlug,
    ExactCwdName,
    ExactTag,
    NameContains,
    SlugContains,
    NoteContains,
    CwdContains,
}

impl ResumeMatchRank {
    fn is_precise(self) -> bool {
        matches!(
            self,
            Self::ExactId
                | Self::PrefixId
                | Self::ExactName
                | Self::ExactSlug
                | Self::ExactCwdName
                | Self::ExactTag
        )
    }
}

#[derive(Debug, Clone)]
struct ResumeCandidate {
    session: SessionSummary,
    rank: ResumeMatchRank,
}

fn resumable_sessions(
    session_store: &SessionStore,
    current_session_id: uuid::Uuid,
) -> Result<Vec<SessionSummary>> {
    Ok(session_store
        .list_sessions()?
        .into_iter()
        .filter(|session| session.id != current_session_id)
        .collect())
}

fn search_sessions(sessions: &[SessionSummary], query: &str) -> Vec<ResumeCandidate> {
    let normalized = query.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }

    let mut matches = sessions
        .iter()
        .filter_map(|session| {
            session_match_rank(session, &normalized).map(|rank| ResumeCandidate {
                session: session.clone(),
                rank,
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| right.session.updated_at_ms.cmp(&left.session.updated_at_ms))
            .then_with(|| left.session.id.cmp(&right.session.id))
    });
    matches
}

fn session_match_rank(session: &SessionSummary, query: &str) -> Option<ResumeMatchRank> {
    let id = session.id.to_string();
    let name = session
        .display_name
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let slug = session
        .slug
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let note = session
        .note
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let cwd = session.cwd.display().to_string().to_ascii_lowercase();
    let cwd_name = session
        .cwd
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let exact_tag = session
        .tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(query));

    if id == query {
        Some(ResumeMatchRank::ExactId)
    } else if id.starts_with(query) {
        Some(ResumeMatchRank::PrefixId)
    } else if !name.is_empty() && name == query {
        Some(ResumeMatchRank::ExactName)
    } else if !slug.is_empty() && slug == query {
        Some(ResumeMatchRank::ExactSlug)
    } else if !cwd_name.is_empty() && cwd_name == query {
        Some(ResumeMatchRank::ExactCwdName)
    } else if exact_tag {
        Some(ResumeMatchRank::ExactTag)
    } else if !name.is_empty() && name.contains(query) {
        Some(ResumeMatchRank::NameContains)
    } else if !slug.is_empty() && slug.contains(query) {
        Some(ResumeMatchRank::SlugContains)
    } else if !note.is_empty() && note.contains(query) {
        Some(ResumeMatchRank::NoteContains)
    } else if cwd.contains(query) {
        Some(ResumeMatchRank::CwdContains)
    } else {
        None
    }
}

fn render_resume_listing(sessions: &[SessionSummary]) -> String {
    if sessions.is_empty() {
        return "No conversations found to resume.".to_string();
    }

    let mut text = String::from("Recent sessions:\n");
    for session in sessions.iter().take(20) {
        let _ = writeln!(&mut text, "{}", format_session_line(session));
    }
    text.push_str("\nRun `/resume <session-id|name|slug|tag>` to restore one session.");
    text
}

fn render_resume_ambiguity(query: &str, matches: &[ResumeCandidate]) -> String {
    let mut text = format!(
        "Found {} sessions matching `{query}`.\nUse `/resume <session-id>` to pick one.\n\nMatches:\n",
        matches.len()
    );
    for candidate in matches.iter().take(10) {
        let _ = writeln!(&mut text, "{}", format_session_line(&candidate.session));
    }
    if matches.len() > 10 {
        let _ = writeln!(&mut text, "... {} more", matches.len() - 10);
    }
    text
}

fn format_session_line(session: &SessionSummary) -> String {
    let mut extras = Vec::new();
    if let Some(slug) = session
        .slug
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        extras.push(format!("slug={slug}"));
    }
    if !session.tags.is_empty() {
        extras.push(format!("tags={}", session.tags.join(",")));
    }
    let extra_suffix = if extras.is_empty() {
        String::new()
    } else {
        format!(" {}", extras.join(" "))
    };
    format!(
        "- {} {} [{}]{}",
        session.id,
        session.display_name.as_deref().unwrap_or("<unnamed>"),
        session.cwd.display(),
        extra_suffix
    )
}

fn resume_into_session(
    state: &mut AppState,
    session_store: &SessionStore,
    summary: &SessionSummary,
) -> Result<()> {
    let record = session_store.load_session(summary.id)?;
    let config = state.config.clone();
    *state = AppState::from_session_record(config, record);
    emit_system(
        state,
        session_store,
        format!(
            "Resumed session {} [{}].",
            state.session.id,
            state.session.display_name.as_deref().unwrap_or("<unnamed>")
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::search_sessions;
    use puffer_session_store::SessionSummary;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn session(
        id: &str,
        name: Option<&str>,
        cwd: &str,
        slug: Option<&str>,
        tags: &[&str],
        note: Option<&str>,
        updated_at_ms: u64,
    ) -> SessionSummary {
        SessionSummary {
            id: Uuid::parse_str(id).unwrap(),
            display_name: name.map(str::to_string),
            cwd: PathBuf::from(cwd),
            created_at_ms: updated_at_ms,
            updated_at_ms,
            event_count: 0,
            parent_session_id: None,
            slug: slug.map(str::to_string),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            note: note.map(str::to_string),
        }
    }

    #[test]
    fn search_prefers_exact_slug_over_note_match() {
        let sessions = vec![
            session(
                "11111111-1111-1111-1111-111111111111",
                Some("Review"),
                "/tmp/one",
                Some("dockyard"),
                &[],
                None,
                1,
            ),
            session(
                "22222222-2222-2222-2222-222222222222",
                Some("Other"),
                "/tmp/two",
                None,
                &[],
                Some("dockyard"),
                2,
            ),
        ];

        let matches = search_sessions(&sessions, "dockyard");
        assert_eq!(matches[0].session.id, sessions[0].id);
    }

    #[test]
    fn search_matches_tags_and_cwd_names() {
        let sessions = vec![
            session(
                "11111111-1111-1111-1111-111111111111",
                Some("Review"),
                "/tmp/shipyard",
                None,
                &["review"],
                None,
                1,
            ),
            session(
                "22222222-2222-2222-2222-222222222222",
                Some("Refactor"),
                "/tmp/dockyard",
                None,
                &[],
                None,
                2,
            ),
        ];

        assert_eq!(
            search_sessions(&sessions, "review")[0].session.id,
            sessions[0].id
        );
        assert_eq!(
            search_sessions(&sessions, "dockyard")[0].session.id,
            sessions[1].id
        );
    }
}
