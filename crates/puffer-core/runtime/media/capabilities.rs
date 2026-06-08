use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Identifies the broad media asset type a capability or job handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MediaKind {
    Image,
    Video,
}

/// Describes one provider/model media generation capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaCapability {
    pub(crate) provider_id: String,
    pub(crate) provider_display_name: String,
    pub(crate) model_id: String,
    pub(crate) model_display_name: String,
    pub(crate) kind: MediaKind,
    pub(crate) operation: String,
    pub(crate) adapter: String,
    pub(crate) parameters: Vec<MediaCapabilityParameter>,
    pub(crate) defaults: BTreeMap<String, String>,
    pub(crate) status: String,
    pub(crate) source: String,
    pub(crate) reason: Option<String>,
    pub(crate) checked_at_ms: u64,
}

/// Describes one select parameter exposed by a media capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaCapabilityParameter {
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) values: Vec<String>,
    pub(crate) default: String,
    pub(crate) request_field: Option<String>,
}
