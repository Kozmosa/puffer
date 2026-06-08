use super::capabilities::MediaCapabilityParameter;
use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// One OpenAI-compatible video generation request (`POST /v1/video/generations`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenAiVideoRequest {
    pub(crate) model: String,
    pub(crate) prompt: String,
    /// Ordered (request_field, value) pairs. A `request_field` of `metadata.<k>`
    /// is nested under the body's `metadata` object; otherwise it is top-level.
    pub(crate) params: Vec<(String, String)>,
}

const METADATA_PREFIX: &str = "metadata.";

impl OpenAiVideoRequest {
    fn validate(&self) -> Result<()> {
        if self.model.trim().is_empty() {
            bail!("video model is required");
        }
        if self.prompt.trim().is_empty() {
            bail!("video prompt is required");
        }
        Ok(())
    }

    /// Builds the `POST /v1/video/generations` request body.
    pub(crate) fn request_body(&self) -> Value {
        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model.trim()));
        body.insert("prompt".to_string(), json!(self.prompt.trim()));
        body.insert("n".to_string(), json!(1));

        let mut metadata = Map::new();
        for (field, value) in &self.params {
            let field = field.trim();
            if let Some(key) = field.strip_prefix(METADATA_PREFIX) {
                metadata.insert(key.to_string(), json!(value.trim()));
            } else {
                body.insert(field.to_string(), json!(value.trim()));
            }
        }
        if !metadata.is_empty() {
            body.insert("metadata".to_string(), Value::Object(metadata));
        }
        Value::Object(body)
    }
}

/// Maps a validated selection's parameters into an OpenAI-video request.
///
/// Emits params in capability order using each parameter's `request_field`
/// (only parameters that declare one). The selected value (already defaulted
/// by the caller) is used, falling back to the parameter default.
pub(crate) fn openai_video_request_from_parameters(
    model_id: String,
    prompt: String,
    capability_parameters: &[MediaCapabilityParameter],
    selected: &BTreeMap<String, String>,
) -> Result<OpenAiVideoRequest> {
    let mut params = Vec::new();
    for parameter in capability_parameters {
        let Some(field) = parameter.request_field.clone() else {
            continue;
        };
        let value = selected
            .get(&parameter.name)
            .cloned()
            .unwrap_or_else(|| parameter.default.clone());
        params.push((field, value));
    }
    let request = OpenAiVideoRequest {
        model: model_id,
        prompt,
        params,
    };
    request.validate()?;
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameter(name: &str, request_field: &str, default: &str) -> MediaCapabilityParameter {
        MediaCapabilityParameter {
            name: name.to_string(),
            label: name.to_string(),
            values: vec![default.to_string()],
            default: default.to_string(),
            request_field: Some(request_field.to_string()),
        }
    }

    #[test]
    fn splits_top_level_and_metadata_params() {
        let params = vec![
            parameter("duration", "seconds", "5"),
            parameter("resolution", "metadata.resolution", "720p"),
            parameter("ratio", "metadata.ratio", "16:9"),
        ];
        let mut selected = BTreeMap::new();
        selected.insert("resolution".to_string(), "1080p".to_string());

        let request = openai_video_request_from_parameters(
            "m".to_string(),
            "a cat".to_string(),
            &params,
            &selected,
        )
        .expect("request");

        let body = request.request_body();
        assert_eq!(body["model"], json!("m"));
        assert_eq!(body["prompt"], json!("a cat"));
        assert_eq!(body["n"], json!(1));
        assert_eq!(body["seconds"], json!("5"));
        assert_eq!(body["metadata"]["resolution"], json!("1080p"));
        assert_eq!(body["metadata"]["ratio"], json!("16:9"));
    }

    #[test]
    fn omits_metadata_when_no_metadata_params() {
        let params = vec![parameter("duration", "seconds", "5")];
        let request = openai_video_request_from_parameters(
            "m".to_string(),
            "a cat".to_string(),
            &params,
            &BTreeMap::new(),
        )
        .expect("request");
        let body = request.request_body();
        assert!(body.get("metadata").is_none());
    }

    #[test]
    fn rejects_empty_prompt() {
        let error = openai_video_request_from_parameters(
            "m".to_string(),
            "   ".to_string(),
            &[],
            &BTreeMap::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("prompt is required"));
    }
}
