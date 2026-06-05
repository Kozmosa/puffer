use crate::runtime::media::{
    MediaArtifact, MediaGenerationService, MediaJob, MediaJobStatus, MediaKind, OpenAIImageRequest,
};
use crate::AppState;
use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use puffer_config::ImageMediaConfig;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const DEFAULT_TIMEOUT_MS: u64 = 300_000;
const MAX_PROMPT_CHARS: usize = 20_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageGenerationInput {
    prompt: String,
    #[serde(default)]
    prompt_reference: Option<String>,
    #[serde(default)]
    aspect: Option<String>,
    #[serde(default)]
    output_path: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    retry_from_error: Option<Value>,
}

#[derive(Debug, PartialEq, Eq)]
struct ImageRequest {
    provider: String,
    model: String,
    prompt: String,
    size: String,
    quality: String,
    output_format: String,
    output_path: PathBuf,
    purpose: Option<String>,
    retry_from_error: Option<Value>,
}

#[derive(Debug, PartialEq, Eq)]
struct ImageGenerationResult {
    job_id: String,
    artifact_id: String,
    path: PathBuf,
    provider: String,
    model: String,
    status: String,
    size: String,
    purpose: Option<String>,
    retry_from_error: bool,
}

/// Generates an image through OpenAI's image API and writes it into the workspace.
pub fn execute_image_generation(state: &mut AppState, cwd: &Path, input: Value) -> Result<String> {
    let parsed: ImageGenerationInput =
        serde_json::from_value(input).context("invalid ImageGeneration input")?;
    let request = build_image_request(cwd, parsed, &state.config.media.image)?;
    let api_key = openai_api_key()?;
    let client = Client::builder()
        .timeout(Duration::from_millis(DEFAULT_TIMEOUT_MS))
        .build()
        .context("build image generation HTTP client")?;
    let service = MediaGenerationService::new(cwd);
    let job_id = Uuid::new_v4().to_string();
    let artifact_id = Uuid::new_v4().to_string();
    let mut job = MediaJob::new(
        job_id.clone(),
        MediaKind::Image,
        request.provider.clone(),
        request.model.clone(),
        request.prompt.clone(),
        now_ms(),
    );
    service.save_job(&job)?;
    job.transition(MediaJobStatus::Running, now_ms())?;
    service.save_job(&job)?;

    let image_bytes = match send_openai_image_request(&client, &api_key, &request) {
        Ok(bytes) => bytes,
        Err(error) => {
            job.error = Some(format!("{error:#}"));
            job.transition(MediaJobStatus::Failed, now_ms())?;
            service.save_job(&job)?;
            return Err(error);
        }
    };

    if let Some(parent) = request.output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create image output directory {}", parent.display()))?;
    }
    fs::write(&request.output_path, &image_bytes)
        .with_context(|| format!("write image output {}", request.output_path.display()))?;
    let artifact = MediaArtifact {
        id: artifact_id.clone(),
        job_id: job_id.clone(),
        kind: MediaKind::Image,
        path: request.output_path.clone(),
        mime_type: mime_type_for_output_format(&request.output_format).to_string(),
        byte_count: image_bytes.len() as u64,
        metadata: json!({
            "size": request.size,
            "quality": request.quality,
            "outputFormat": request.output_format,
            "retryFromError": request.retry_from_error.is_some()
        }),
        created_at_ms: now_ms(),
    };
    service.save_artifact(&artifact)?;
    job.attach_artifact(artifact_id.clone(), now_ms());
    job.transition(MediaJobStatus::Succeeded, now_ms())?;
    service.save_job(&job)?;

    image_generation_output(&ImageGenerationResult {
        job_id,
        artifact_id,
        path: request.output_path,
        provider: request.provider,
        model: request.model,
        status: "succeeded".to_string(),
        size: request.size,
        purpose: request.purpose,
        retry_from_error: request.retry_from_error.is_some(),
    })
}

fn send_openai_image_request(
    client: &Client,
    api_key: &str,
    request: &ImageRequest,
) -> Result<Vec<u8>> {
    let response = client
        .post("https://api.openai.com/v1/images/generations")
        .bearer_auth(api_key)
        .json(&openai_image_request_body(request))
        .send()
        .context("send image generation request")?;
    let status = response.status();
    let body = response.text().context("read image generation response")?;
    if !status.is_success() {
        bail!(
            "image generation failed with status {}: {}",
            status.as_u16(),
            body
        );
    }
    let value: Value = serde_json::from_str(&body).context("parse image generation response")?;
    image_bytes_from_response(client, &value)
}

fn build_image_request(
    cwd: &Path,
    input: ImageGenerationInput,
    settings: &ImageMediaConfig,
) -> Result<ImageRequest> {
    let prompt = prompt_text(cwd, &input.prompt, input.prompt_reference.as_deref())?;
    let provider = required_media_setting(settings.provider_id.as_deref(), "providerId")?;
    let model = required_media_setting(settings.model_id.as_deref(), "modelId")?;
    if provider != "openai" {
        bail!("ImageGeneration media provider `{provider}` is not supported");
    }
    let output_format = normalized_output_format(&settings.output_format)?;
    Ok(ImageRequest {
        provider,
        model,
        prompt,
        size: if input
            .aspect
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        {
            image_size(input.aspect.as_deref())?.to_string()
        } else {
            non_empty_media_value(&settings.size, "size")?
        },
        quality: non_empty_media_value(&settings.quality, "quality")?,
        output_path: resolve_output_path(cwd, input.output_path.as_deref(), &output_format)?,
        output_format,
        purpose: input.purpose,
        retry_from_error: input.retry_from_error,
    })
}

fn required_media_setting(value: Option<&str>, field: &str) -> Result<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .with_context(|| format!("ImageGeneration media image {field} is not configured"))
}

fn non_empty_media_value(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("ImageGeneration media image {field} is not configured");
    }
    Ok(trimmed.to_string())
}

fn normalized_output_format(value: &str) -> Result<String> {
    let format = non_empty_media_value(value, "outputFormat")?;
    match format.to_ascii_lowercase().as_str() {
        "png" => Ok("png".to_string()),
        "jpg" | "jpeg" => Ok("jpeg".to_string()),
        "webp" => Ok("webp".to_string()),
        other => bail!("unsupported ImageGeneration media image outputFormat `{other}`"),
    }
}

fn openai_image_request_body(request: &ImageRequest) -> Value {
    OpenAIImageRequest::new(
        &request.model,
        &request.prompt,
        &request.size,
        &request.quality,
        &request.output_format,
    )
    .to_body()
}

fn image_generation_output(result: &ImageGenerationResult) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "jobId": result.job_id,
        "artifactId": result.artifact_id,
        "path": result.path,
        "provider": result.provider,
        "model": result.model,
        "status": result.status,
        "size": result.size,
        "purpose": result.purpose,
        "retryFromError": result.retry_from_error
    }))?)
}

fn mime_type_for_output_format(format: &str) -> &'static str {
    match format.trim().to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn prompt_text(cwd: &Path, value: &str, reference: Option<&str>) -> Result<String> {
    let primary = prompt_fragment(cwd, value, "prompt")?;
    let Some(reference) = reference.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(primary);
    };
    let reference = prompt_fragment(cwd, reference, "promptReference")?;
    let prompt = format!("Reference prompt document:\n{reference}\n\nImage prompt:\n{primary}");
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        bail!("ImageGeneration prompt exceeds {MAX_PROMPT_CHARS} characters");
    }
    Ok(prompt)
}

fn prompt_fragment(cwd: &Path, value: &str, field: &str) -> Result<String> {
    let text = value.trim();
    if text.is_empty() {
        bail!("ImageGeneration `{field}` is required");
    }
    let candidate = cwd.join(text);
    let prompt = if safe_relative_path(text) && candidate.is_file() {
        fs::read_to_string(&candidate)
            .with_context(|| format!("read ImageGeneration `{field}` {}", candidate.display()))?
    } else {
        text.to_string()
    };
    let prompt = prompt.trim();
    if prompt.is_empty() {
        bail!("ImageGeneration `{field}` is empty");
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        bail!("ImageGeneration prompt exceeds {MAX_PROMPT_CHARS} characters");
    }
    Ok(prompt.to_string())
}

fn image_size(aspect: Option<&str>) -> Result<&'static str> {
    let Some(aspect) = aspect.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok("1024x1024");
    };
    match aspect.to_ascii_lowercase().as_str() {
        "square" | "1:1" | "1024x1024" => Ok("1024x1024"),
        "landscape" | "wide" | "horizontal" | "16:9" | "3:2" | "1536x1024" => Ok("1536x1024"),
        "portrait" | "vertical" | "9:16" | "2:3" | "1024x1536" => Ok("1024x1536"),
        "auto" => Ok("auto"),
        other => bail!("unsupported ImageGeneration aspect `{other}`"),
    }
}

fn resolve_output_path(cwd: &Path, value: Option<&str>, output_format: &str) -> Result<PathBuf> {
    let relative = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default_output_name(output_format));
    if !safe_relative_path(&relative) {
        bail!("ImageGeneration outputPath must be a safe relative path");
    }
    Ok(cwd.join(relative))
}

fn default_output_name(output_format: &str) -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!(
        ".puffer/workflows/images/generated-{stamp}.{}",
        extension_for_output_format(output_format)
    )
}

fn extension_for_output_format(format: &str) -> &'static str {
    match format.trim().to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => "jpeg",
        "webp" => "webp",
        _ => "png",
    }
}

fn safe_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn image_bytes_from_response(client: &Client, value: &Value) -> Result<Vec<u8>> {
    let first = value
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .context("image generation response missing data[0]")?;
    if let Some(encoded) = first.get("b64_json").and_then(Value::as_str) {
        return BASE64_STANDARD
            .decode(encoded.trim())
            .context("decode image b64_json");
    }
    if let Some(url) = first.get("url").and_then(Value::as_str) {
        return download_image_url(client, url);
    }
    bail!("image generation response missing b64_json or url")
}

fn download_image_url(client: &Client, url: &str) -> Result<Vec<u8>> {
    let parsed = reqwest::Url::parse(url).context("image response URL must be absolute")?;
    match parsed.scheme() {
        "https" => {}
        other => bail!("unsupported image response URL scheme `{other}`"),
    }
    let response = client
        .get(parsed)
        .send()
        .context("download generated image")?;
    let status = response.status();
    if !status.is_success() {
        bail!(
            "download generated image failed with status {}",
            status.as_u16()
        );
    }
    Ok(response
        .bytes()
        .context("read generated image bytes")?
        .to_vec())
}

fn openai_api_key() -> Result<String> {
    std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("PUFFER_OPENAI_API_KEY"))
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .context("ImageGeneration requires OPENAI_API_KEY or PUFFER_OPENAI_API_KEY")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn image_settings() -> ImageMediaConfig {
        ImageMediaConfig {
            provider_id: Some("openai".to_string()),
            model_id: Some("gpt-image-1".to_string()),
            size: "1024x1024".to_string(),
            quality: "auto".to_string(),
            output_format: "png".to_string(),
        }
    }

    #[test]
    fn maps_common_aspects_to_image_sizes() {
        assert_eq!(image_size(None).unwrap(), "1024x1024");
        assert_eq!(image_size(Some("landscape")).unwrap(), "1536x1024");
        assert_eq!(image_size(Some("portrait")).unwrap(), "1024x1536");
        assert_eq!(image_size(Some("auto")).unwrap(), "auto");
        assert!(image_size(Some("panorama")).is_err());
    }

    #[test]
    fn reads_prompt_from_safe_workspace_relative_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("prompt.md"), "draw a careful diagram").unwrap();

        assert_eq!(
            prompt_text(dir.path(), "prompt.md", None).unwrap(),
            "draw a careful diagram"
        );
    }

    #[test]
    fn combines_prompt_reference_with_primary_prompt() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("prompts.md"), "character guide").unwrap();

        let prompt = prompt_text(dir.path(), "panel 1 action", Some("prompts.md")).unwrap();

        assert!(prompt.contains("character guide"));
        assert!(prompt.contains("panel 1 action"));
    }

    #[test]
    fn parses_prompt_reference_from_tool_input() {
        let parsed: ImageGenerationInput = serde_json::from_value(json!({
            "prompt": "panel 1 action",
            "promptReference": "prompts.md"
        }))
        .unwrap();

        assert_eq!(parsed.prompt_reference.as_deref(), Some("prompts.md"));
    }

    #[test]
    fn rejects_unsafe_output_paths() {
        let dir = tempdir().unwrap();

        assert!(resolve_output_path(dir.path(), Some("../out.png"), "png").is_err());
        assert!(resolve_output_path(dir.path(), Some("/tmp/out.png"), "png").is_err());
        assert!(resolve_output_path(dir.path(), Some("images/out.png"), "png").is_ok());
    }

    #[test]
    fn default_output_path_uses_media_output_format_extension() {
        let dir = tempdir().unwrap();
        let settings = puffer_config::ImageMediaConfig {
            provider_id: Some("openai".to_string()),
            model_id: Some("gpt-image-1".to_string()),
            size: "1024x1024".to_string(),
            quality: "auto".to_string(),
            output_format: "webp".to_string(),
        };

        let request = build_image_request(
            dir.path(),
            ImageGenerationInput {
                prompt: "make a visual summary".to_string(),
                prompt_reference: None,
                aspect: None,
                output_path: None,
                purpose: None,
                retry_from_error: None,
            },
            &settings,
        )
        .unwrap();

        assert_eq!(
            request
                .output_path
                .extension()
                .and_then(|value| value.to_str()),
            Some("webp")
        );
    }

    #[test]
    fn extracts_base64_image_bytes() {
        let client = Client::new();
        let value = json!({
            "data": [
                {
                    "b64_json": "AAECAw=="
                }
            ]
        });

        assert_eq!(
            image_bytes_from_response(&client, &value).unwrap(),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn builds_request_with_prompt_file_and_output() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("prompt.md"), "make a visual summary").unwrap();

        let request = build_image_request(
            dir.path(),
            ImageGenerationInput {
                prompt: "prompt.md".to_string(),
                prompt_reference: None,
                aspect: Some("square".to_string()),
                output_path: Some("out/image.png".to_string()),
                purpose: Some("test".to_string()),
                retry_from_error: None,
            },
            &image_settings(),
        )
        .unwrap();

        assert_eq!(request.prompt, "make a visual summary");
        assert_eq!(request.size, "1024x1024");
        assert_eq!(request.output_path, dir.path().join("out/image.png"));
    }

    #[test]
    fn builds_request_from_media_settings_instead_of_env_model() {
        let dir = tempdir().unwrap();
        std::env::set_var("PUFFER_IMAGE_MODEL", "legacy-env-model");
        let settings = puffer_config::ImageMediaConfig {
            provider_id: Some("openai".to_string()),
            model_id: Some("configured-image-model".to_string()),
            size: "1024x1024".to_string(),
            quality: "high".to_string(),
            output_format: "webp".to_string(),
        };

        let request = build_image_request(
            dir.path(),
            ImageGenerationInput {
                prompt: "make a visual summary".to_string(),
                prompt_reference: None,
                aspect: None,
                output_path: Some("out/image.webp".to_string()),
                purpose: None,
                retry_from_error: None,
            },
            &settings,
        )
        .unwrap();

        assert_eq!(request.provider, "openai");
        assert_eq!(request.model, "configured-image-model");
        assert_eq!(request.quality, "high");
        assert_eq!(request.output_format, "webp");
        std::env::remove_var("PUFFER_IMAGE_MODEL");
    }

    #[test]
    fn openai_adapter_request_body_uses_media_request_shape() {
        let body = openai_image_request_body(&ImageRequest {
            provider: "openai".to_string(),
            model: "gpt-image-1".to_string(),
            prompt: "draw a careful diagram".to_string(),
            size: "1536x1024".to_string(),
            quality: "high".to_string(),
            output_format: "png".to_string(),
            output_path: PathBuf::from("out.png"),
            purpose: None,
            retry_from_error: None,
        });

        assert_eq!(body["model"], "gpt-image-1");
        assert_eq!(body["prompt"], "draw a careful diagram");
        assert_eq!(body["size"], "1536x1024");
        assert_eq!(body["quality"], "high");
        assert_eq!(body["output_format"], "png");
    }

    #[test]
    fn image_generation_output_includes_job_and_artifact_metadata() {
        let output = image_generation_output(&ImageGenerationResult {
            job_id: "job-1".to_string(),
            artifact_id: "artifact-1".to_string(),
            path: PathBuf::from("out/image.png"),
            provider: "openai".to_string(),
            model: "gpt-image-1".to_string(),
            status: "succeeded".to_string(),
            size: "1024x1024".to_string(),
            purpose: Some("test".to_string()),
            retry_from_error: false,
        })
        .unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(parsed["jobId"], "job-1");
        assert_eq!(parsed["artifactId"], "artifact-1");
        assert_eq!(parsed["provider"], "openai");
        assert_eq!(parsed["model"], "gpt-image-1");
        assert_eq!(parsed["status"], "succeeded");
    }
}
