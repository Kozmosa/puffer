use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Request shape for OpenAI image generation after media settings resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenAIImageRequest {
    pub(crate) model: String,
    pub(crate) prompt: String,
    pub(crate) size: String,
    pub(crate) quality: String,
    pub(crate) output_format: String,
}

impl OpenAIImageRequest {
    /// Creates a normalized OpenAI image generation request.
    pub(crate) fn new(
        model: impl Into<String>,
        prompt: impl Into<String>,
        size: impl Into<String>,
        quality: impl Into<String>,
        output_format: impl Into<String>,
    ) -> Self {
        Self {
            model: model.into(),
            prompt: prompt.into(),
            size: size.into(),
            quality: quality.into(),
            output_format: output_format.into(),
        }
    }

    /// Converts the request into the OpenAI Images API JSON body.
    pub(crate) fn to_body(&self) -> Value {
        json!({
            "model": self.model,
            "prompt": self.prompt,
            "size": self.size,
            "quality": self.quality,
            "output_format": self.output_format,
            "n": 1
        })
    }
}
