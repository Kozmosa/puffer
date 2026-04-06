use puffer_config::PufferConfig;
use puffer_session_store::{
    ClaudeReadSnapshotEvent, SessionMetadata, SessionRecord, TranscriptEvent, TranscriptRewrite,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Describes the role of a rendered transcript message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

/// Represents one rendered transcript message in the interactive UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenderedMessage {
    pub role: MessageRole,
    pub text: String,
}

/// Describes the completion state of one recorded task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TaskStatus {
    Completed,
    Failed,
}

/// Represents one recorded shell or tool task in the current session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskRecord {
    pub id: u64,
    pub label: String,
    pub detail: String,
    pub status: TaskStatus,
}

/// Stores the last successful Claude-style read metadata for one absolute path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaudeReadState {
    pub(crate) timestamp_ms: u128,
    pub(crate) is_partial_view: bool,
}

/// Stores the mutable session and UI state for one interactive Puffer run.
#[derive(Debug, Clone)]
pub struct AppState {
    pub config: PufferConfig,
    pub cwd: PathBuf,
    pub working_dirs: Vec<PathBuf>,
    pub session: SessionMetadata,
    pub transcript: Vec<RenderedMessage>,
    pub current_model: Option<String>,
    pub current_provider: Option<String>,
    pub prompt_color: String,
    pub effort_level: String,
    pub fast_mode: bool,
    pub plan_mode: bool,
    pub sandbox_mode: String,
    pub remote_name: Option<String>,
    pub remote_environment: Option<String>,
    pub remote_session_id: Option<String>,
    pub remote_session_url: Option<String>,
    pub remote_session_status: Option<String>,
    pub statusline_enabled: bool,
    pub vim_mode: bool,
    pub should_exit: bool,
    pub reload_resources_requested: bool,
    pub(crate) claude_read_state: HashMap<PathBuf, ClaudeReadState>,
    tasks: Vec<TaskRecord>,
    next_task_id: u64,
}

impl AppState {
    /// Creates a new application state for the active session.
    pub fn new(config: PufferConfig, cwd: PathBuf, session: SessionMetadata) -> Self {
        Self {
            current_model: config.default_model.clone(),
            current_provider: config.default_provider.clone(),
            config,
            cwd,
            working_dirs: Vec::new(),
            session,
            transcript: Vec::new(),
            prompt_color: "default".to_string(),
            effort_level: "medium".to_string(),
            fast_mode: false,
            plan_mode: false,
            sandbox_mode: "workspace-write".to_string(),
            remote_name: None,
            remote_environment: None,
            remote_session_id: None,
            remote_session_url: None,
            remote_session_status: None,
            statusline_enabled: true,
            vim_mode: false,
            should_exit: false,
            reload_resources_requested: false,
            claude_read_state: HashMap::new(),
            tasks: Vec::new(),
            next_task_id: 1,
        }
    }

    /// Restores application state from a persisted session record.
    pub fn from_session_record(config: PufferConfig, session: SessionRecord) -> Self {
        let cwd = session.metadata.cwd.clone();
        let mut state = Self::new(config, cwd, session.metadata);
        for event in session.events {
            match event {
                TranscriptEvent::UserMessage { text } => {
                    state.push_message(MessageRole::User, text)
                }
                TranscriptEvent::AssistantMessage { text } => {
                    state.push_message(MessageRole::Assistant, text)
                }
                TranscriptEvent::SystemMessage { text } => {
                    state.push_message(MessageRole::System, text)
                }
                TranscriptEvent::CommandInvoked { .. } => {}
                TranscriptEvent::SessionRenamed { name } => {
                    state.session.display_name = Some(name);
                }
                TranscriptEvent::GitDiffSnapshot { .. } => {}
                TranscriptEvent::TranscriptRewritten { rewrite } => {
                    state.apply_transcript_rewrite(&rewrite);
                }
                TranscriptEvent::StateSnapshot {
                    current_model,
                    current_provider,
                    theme,
                    prompt_color,
                    effort_level,
                    fast_mode,
                    plan_mode,
                    sandbox_mode,
                    remote_name,
                    remote_environment,
                    remote_session_id,
                    remote_session_url,
                    remote_session_status,
                    statusline_enabled,
                    working_dirs,
                    claude_read_state,
                } => {
                    state.current_model = current_model;
                    state.current_provider = current_provider;
                    state.config.theme = theme;
                    state.prompt_color = prompt_color;
                    state.effort_level = effort_level;
                    state.fast_mode = fast_mode;
                    state.plan_mode = plan_mode;
                    state.sandbox_mode = sandbox_mode;
                    state.remote_name = remote_name;
                    state.remote_environment = remote_environment;
                    state.remote_session_id = remote_session_id;
                    state.remote_session_url = remote_session_url;
                    state.remote_session_status = remote_session_status;
                    state.statusline_enabled = statusline_enabled;
                    state.working_dirs = working_dirs.into_iter().map(Into::into).collect();
                    state.claude_read_state = claude_read_state
                        .into_iter()
                        .map(|entry| {
                            (
                                PathBuf::from(entry.path),
                                ClaudeReadState {
                                    timestamp_ms: entry.timestamp_ms,
                                    is_partial_view: entry.is_partial_view,
                                },
                            )
                        })
                        .collect();
                }
            }
        }
        state
    }

    /// Appends a rendered message to the in-memory transcript.
    pub fn push_message(&mut self, role: MessageRole, text: impl Into<String>) {
        self.transcript.push(RenderedMessage {
            role,
            text: text.into(),
        });
    }

    /// Applies one transcript rewrite operation to in-memory transcript state.
    pub fn apply_transcript_rewrite(&mut self, rewrite: &TranscriptRewrite) {
        match rewrite {
            TranscriptRewrite::Clear => self.transcript.clear(),
            TranscriptRewrite::PopLast { count } => {
                for _ in 0..*count {
                    if self.transcript.pop().is_none() {
                        break;
                    }
                }
            }
        }
    }

    /// Records one completed or failed task in the current runtime session state.
    pub fn record_task(
        &mut self,
        label: impl Into<String>,
        detail: impl Into<String>,
        success: bool,
    ) {
        let task = TaskRecord {
            id: self.next_task_id,
            label: label.into(),
            detail: detail.into(),
            status: if success {
                TaskStatus::Completed
            } else {
                TaskStatus::Failed
            },
        };
        self.next_task_id += 1;
        self.tasks.push(task);
    }

    /// Builds a persisted snapshot event for the current mutable session state.
    pub fn snapshot_event(&self) -> TranscriptEvent {
        TranscriptEvent::StateSnapshot {
            current_model: self.current_model.clone(),
            current_provider: self.current_provider.clone(),
            theme: self.config.theme.clone(),
            prompt_color: self.prompt_color.clone(),
            effort_level: self.effort_level.clone(),
            fast_mode: self.fast_mode,
            plan_mode: self.plan_mode,
            sandbox_mode: self.sandbox_mode.clone(),
            remote_name: self.remote_name.clone(),
            remote_environment: self.remote_environment.clone(),
            remote_session_id: self.remote_session_id.clone(),
            remote_session_url: self.remote_session_url.clone(),
            remote_session_status: self.remote_session_status.clone(),
            statusline_enabled: self.statusline_enabled,
            working_dirs: self
                .working_dirs
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            claude_read_state: self
                .claude_read_state
                .iter()
                .map(|(path, snapshot)| ClaudeReadSnapshotEvent {
                    path: path.display().to_string(),
                    timestamp_ms: snapshot.timestamp_ms,
                    is_partial_view: snapshot.is_partial_view,
                })
                .collect(),
        }
    }

    pub(crate) fn tasks(&self) -> &[TaskRecord] {
        &self.tasks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_session_store::TranscriptEvent;
    use uuid::Uuid;

    fn sample_metadata() -> SessionMetadata {
        SessionMetadata {
            id: Uuid::new_v4(),
            display_name: Some("sample".to_string()),
            cwd: PathBuf::from("."),
            created_at_ms: 0,
            updated_at_ms: 0,
            parent_session_id: None,
            slug: None,
            tags: Vec::new(),
            note: None,
        }
    }

    #[test]
    fn session_restore_replays_transcript_rewrite_events() {
        let record = SessionRecord {
            metadata: sample_metadata(),
            events: vec![
                TranscriptEvent::UserMessage {
                    text: "u1".to_string(),
                },
                TranscriptEvent::AssistantMessage {
                    text: "a1".to_string(),
                },
                TranscriptEvent::TranscriptRewritten {
                    rewrite: TranscriptRewrite::PopLast { count: 1 },
                },
                TranscriptEvent::SystemMessage {
                    text: "after-pop".to_string(),
                },
                TranscriptEvent::TranscriptRewritten {
                    rewrite: TranscriptRewrite::Clear,
                },
                TranscriptEvent::SystemMessage {
                    text: "after-clear".to_string(),
                },
            ],
        };
        let state = AppState::from_session_record(PufferConfig::default(), record);
        assert_eq!(state.transcript.len(), 1);
        assert_eq!(state.transcript[0].role, MessageRole::System);
        assert_eq!(state.transcript[0].text, "after-clear");
    }

    #[test]
    fn session_restore_ignores_command_invoked_for_transcript_reconstruction() {
        let record = SessionRecord {
            metadata: sample_metadata(),
            events: vec![
                TranscriptEvent::UserMessage {
                    text: "before".to_string(),
                },
                TranscriptEvent::CommandInvoked {
                    name: "help".to_string(),
                    args: String::new(),
                },
                TranscriptEvent::SystemMessage {
                    text: "done".to_string(),
                },
            ],
        };
        let state = AppState::from_session_record(PufferConfig::default(), record);
        let lines = state
            .transcript
            .iter()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(lines, vec!["before", "done"]);
    }
}
