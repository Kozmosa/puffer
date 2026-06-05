use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    pub(crate) model_id: String,
    pub(crate) kind: MediaKind,
    pub(crate) operations: Vec<String>,
    pub(crate) supports_async: bool,
    pub(crate) supports_streaming: bool,
    pub(crate) parameter_values: Value,
    pub(crate) status: String,
    pub(crate) source: String,
    pub(crate) reason: Option<String>,
    pub(crate) checked_at_ms: u64,
}

/// Stores image generation defaults resolved before adapter execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImageDefaults {
    pub(crate) provider_id: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) size: String,
    pub(crate) quality: String,
    pub(crate) output_format: String,
}

impl Default for ImageDefaults {
    fn default() -> Self {
        Self {
            provider_id: None,
            model_id: None,
            size: "1024x1024".to_string(),
            quality: "auto".to_string(),
            output_format: "png".to_string(),
        }
    }
}

/// Stores video generation defaults resolved before adapter execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoDefaults {
    pub(crate) provider_id: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) aspect_ratio: String,
    pub(crate) duration_seconds: u32,
}

impl Default for VideoDefaults {
    fn default() -> Self {
        Self {
            provider_id: None,
            model_id: None,
            aspect_ratio: "16:9".to_string(),
            duration_seconds: 8,
        }
    }
}
