use super::capabilities::MediaKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Stores durable metadata for a generated media file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaArtifact {
    pub(crate) id: String,
    pub(crate) job_id: String,
    pub(crate) kind: MediaKind,
    pub(crate) path: PathBuf,
    pub(crate) mime_type: String,
    pub(crate) byte_count: u64,
    pub(crate) metadata: Value,
    pub(crate) created_at_ms: u64,
}
