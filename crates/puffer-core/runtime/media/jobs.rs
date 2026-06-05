use super::capabilities::MediaKind;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Tracks normalized media generation job states across providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MediaJobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

impl MediaJobStatus {
    /// Returns true when no further provider polling or state transition is expected.
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Canceled)
    }
}

/// Stores durable job metadata for one media generation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaJob {
    pub(crate) id: String,
    pub(crate) kind: MediaKind,
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) prompt: String,
    pub(crate) status: MediaJobStatus,
    pub(crate) provider_job_id: Option<String>,
    pub(crate) remote_status: Option<String>,
    pub(crate) remote_get_url: Option<String>,
    pub(crate) remote_cancel_url: Option<String>,
    pub(crate) artifact_ids: Vec<String>,
    pub(crate) error: Option<String>,
    pub(crate) created_at_ms: u64,
    pub(crate) updated_at_ms: u64,
}

impl MediaJob {
    /// Creates a queued media job with provider and prompt metadata.
    pub(crate) fn new(
        id: impl Into<String>,
        kind: MediaKind,
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        prompt: impl Into<String>,
        now_ms: u64,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            provider_id: provider_id.into(),
            model_id: model_id.into(),
            prompt: prompt.into(),
            status: MediaJobStatus::Queued,
            provider_job_id: None,
            remote_status: None,
            remote_get_url: None,
            remote_cancel_url: None,
            artifact_ids: Vec::new(),
            error: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
    }

    /// Moves the job to a new state unless it would leave a terminal state.
    pub(crate) fn transition(&mut self, next: MediaJobStatus, now_ms: u64) -> Result<()> {
        if self.status.is_terminal() && self.status != next {
            bail!("cannot transition terminal media job `{}`", self.id);
        }
        self.status = next;
        self.updated_at_ms = now_ms;
        Ok(())
    }

    /// Attaches a generated artifact id to the job without duplicating ids.
    pub(crate) fn attach_artifact(&mut self, artifact_id: impl Into<String>, now_ms: u64) {
        let artifact_id = artifact_id.into();
        if !self.artifact_ids.iter().any(|id| id == &artifact_id) {
            self.artifact_ids.push(artifact_id);
        }
        self.updated_at_ms = now_ms;
    }
}
