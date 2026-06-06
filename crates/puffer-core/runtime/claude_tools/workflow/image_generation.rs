use crate::AppState;
use crate::{
    generate_exact_image_with_cache, resolved_exact_image_parameters_with_cache,
    ExactImageGenerationRequest, ExactMediaDiscoveryCache,
};
use anyhow::{bail, Context, Result};
use puffer_config::ImageMediaConfig;
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_PROMPT_CHARS: usize = 20_000;

/// Carries exact media runtime context into the ImageGeneration workflow tool.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ImageGenerationMediaContext<'a> {
    pub(crate) providers: &'a ProviderRegistry,
    pub(crate) auth_store: &'a AuthStore,
    pub(crate) discovery_cache: &'a ExactMediaDiscoveryCache,
}

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
    adapter: String,
    prompt: String,
    parameters: BTreeMap<String, String>,
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
    parameters: BTreeMap<String, String>,
    purpose: Option<String>,
    retry_from_error: bool,
}

/// Generates an image through the exact media runtime and writes it into the workspace.
pub fn execute_image_generation(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
    media_context: Option<ImageGenerationMediaContext<'_>>,
) -> Result<String> {
    let parsed: ImageGenerationInput =
        serde_json::from_value(input).context("invalid ImageGeneration input")?;
    let mut request = build_image_request(cwd, parsed, &state.config.media.image)?;
    let media_context = media_context.context("ImageGeneration media runtime is not configured")?;
    request.parameters = resolved_exact_image_parameters_with_cache(
        media_context.providers,
        media_context.auth_store,
        &ExactImageGenerationRequest {
            provider_id: request.provider.clone(),
            model_id: request.model.clone(),
            adapter: request.adapter.clone(),
            prompt: request.prompt.clone(),
            parameters: request.parameters.clone(),
        },
        media_context.discovery_cache,
    )?;
    let generated = generate_exact_image_with_cache(
        media_context.providers,
        media_context.auth_store,
        cwd,
        ExactImageGenerationRequest {
            provider_id: request.provider.clone(),
            model_id: request.model.clone(),
            adapter: request.adapter.clone(),
            prompt: request.prompt.clone(),
            parameters: request.parameters.clone(),
        },
        media_context.discovery_cache,
    )?;

    if let Some(parent) = request.output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create image output directory {}", parent.display()))?;
    }
    fs::copy(&generated.path, &request.output_path)
        .with_context(|| format!("write image output {}", request.output_path.display()))?;

    image_generation_output(&ImageGenerationResult {
        job_id: generated.job_id,
        artifact_id: generated.artifact_id,
        path: request.output_path,
        provider: generated.provider_id,
        model: generated.model_id,
        status: generated.status,
        parameters: request.parameters,
        purpose: request.purpose,
        retry_from_error: request.retry_from_error.is_some(),
    })
}

fn build_image_request(
    cwd: &Path,
    input: ImageGenerationInput,
    settings: &ImageMediaConfig,
) -> Result<ImageRequest> {
    let prompt = prompt_text(cwd, &input.prompt, input.prompt_reference.as_deref())?;
    let (provider, model, adapter) = required_provider_model_adapter(settings)?;
    let mut parameters = settings.parameters.clone();
    apply_aspect_parameter(&mut parameters, input.aspect.as_deref())?;
    let output_format = normalized_output_format(
        parameters
            .get("output_format")
            .map(String::as_str)
            .unwrap_or("png"),
    )?;
    Ok(ImageRequest {
        provider,
        model,
        adapter,
        prompt,
        parameters,
        output_path: resolve_output_path(cwd, input.output_path.as_deref(), &output_format)?,
        output_format,
        purpose: input.purpose,
        retry_from_error: input.retry_from_error,
    })
}

fn required_provider_model_adapter(
    settings: &ImageMediaConfig,
) -> Result<(String, String, String)> {
    let provider = settings
        .provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let model = settings
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let adapter = settings
        .adapter
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (provider, model, adapter) {
        (Some(provider), Some(model), Some(adapter)) => {
            Ok((provider.to_string(), model.to_string(), adapter.to_string()))
        }
        _ => bail!("image media provider/model/adapter is not configured"),
    }
}

fn non_empty_media_value(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("ImageGeneration media image {field} is not configured");
    }
    Ok(trimmed.to_string())
}

fn normalized_output_format(value: &str) -> Result<String> {
    let format = non_empty_media_value(value, "output_format")?;
    match format.to_ascii_lowercase().as_str() {
        "png" => Ok("png".to_string()),
        "jpg" | "jpeg" => Ok("jpeg".to_string()),
        "webp" => Ok("webp".to_string()),
        other => bail!("unsupported ImageGeneration media image output_format `{other}`"),
    }
}

fn image_generation_output(result: &ImageGenerationResult) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "jobId": result.job_id,
        "artifactId": result.artifact_id,
        "path": result.path,
        "provider": result.provider,
        "model": result.model,
        "status": result.status,
        "parameters": result.parameters,
        "purpose": result.purpose,
        "retryFromError": result.retry_from_error
    }))?)
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

fn image_aspect_ratio(aspect: &str) -> Result<&'static str> {
    match aspect.trim().to_ascii_lowercase().as_str() {
        "square" | "1:1" | "1024x1024" => Ok("1:1"),
        "landscape" | "wide" | "horizontal" | "16:9" | "1536x1024" => Ok("16:9"),
        "portrait" | "vertical" | "9:16" | "1024x1536" => Ok("9:16"),
        "3:2" => Ok("3:2"),
        "2:3" => Ok("2:3"),
        "4:3" => Ok("4:3"),
        "3:4" => Ok("3:4"),
        "21:9" => Ok("21:9"),
        "auto" => Ok("auto"),
        other => bail!("unsupported ImageGeneration aspect `{other}`"),
    }
}

fn is_dimension_size(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1024x1024" | "1536x1024" | "1024x1536" | "auto"
    )
}

fn apply_aspect_parameter(
    parameters: &mut BTreeMap<String, String>,
    aspect: Option<&str>,
) -> Result<()> {
    let Some(aspect) = aspect.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if parameters.contains_key("aspect_ratio") {
        parameters.insert(
            "aspect_ratio".to_string(),
            image_aspect_ratio(aspect)?.to_string(),
        );
        return Ok(());
    }
    if let Some(current_size) = parameters.get("size") {
        if is_dimension_size(current_size) {
            parameters.insert("size".to_string(), image_size(Some(aspect))?.to_string());
        }
        return Ok(());
    }
    bail!("selected image model does not support ImageGeneration aspect")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::profile::{EffectiveApprovalPolicy, EffectiveSandboxMode};
    use crate::permissions::FilesystemPermissionPolicy;
    use crate::runtime::claude_tools::{execute_tool, ProviderToolContext};
    use indexmap::IndexMap;
    use puffer_provider_registry::{
        AuthMode, AuthStore, ImageMediaDescriptor, MediaExecutionDescriptor, MediaExecutionKind,
        MediaModelDescriptor, MediaOperation, MediaParameterSpec, ModelDescriptor,
        ProviderDescriptor, ProviderMediaDescriptor, ProviderRegistry,
    };
    use puffer_resources::LoadedResources;
    use puffer_session_store::SessionMetadata;
    use puffer_tools::{
        ToolDefinition, ToolDisplayHints, ToolInputSchema, ToolKind, ToolMetadata, ToolPolicyHints,
        ToolRegistry,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn image_settings() -> ImageMediaConfig {
        ImageMediaConfig {
            provider_id: Some("openai".to_string()),
            model_id: Some("gpt-image-1".to_string()),
            adapter: Some("images_json".to_string()),
            parameters: BTreeMap::from([
                ("size".to_string(), "1024x1024".to_string()),
                ("quality".to_string(), "auto".to_string()),
                ("output_format".to_string(), "png".to_string()),
            ]),
        }
    }

    fn test_state(settings: ImageMediaConfig, cwd: &Path) -> AppState {
        let mut config = puffer_config::PufferConfig::default();
        config.media.image = settings;
        AppState::new(
            config,
            cwd.to_path_buf(),
            SessionMetadata {
                id: Uuid::new_v4(),
                display_name: None,
                generated_title: None,
                cwd: cwd.to_path_buf(),
                created_at_ms: 0,
                updated_at_ms: 0,
                parent_session_id: None,
                slug: None,
                tags: Vec::new(),
                note: None,
            },
        )
    }

    fn registry_with_provider(base_url: String) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderDescriptor {
            id: "exact-provider".to_string(),
            display_name: "Exact Provider".to_string(),
            base_url,
            default_api: "openai-responses".to_string(),
            auth_modes: vec![AuthMode::ApiKey],
            headers: IndexMap::new(),
            query_params: IndexMap::new(),
            chat_completions_path: None,
            discovery: None,
            media: Some(ProviderMediaDescriptor {
                image: Some(ImageMediaDescriptor {
                    discovery: None,
                    execution: Some(MediaExecutionDescriptor {
                        adapter: MediaExecutionKind::ImagesJson,
                        base_url: None,
                        path: "/custom/images".to_string(),
                    }),
                    models: vec![MediaModelDescriptor {
                        id: "exact-image-model".to_string(),
                        display_name: Some("Exact Image Model".to_string()),
                        execution: None,
                        operations: vec![MediaOperation::Generate],
                        parameters: vec![
                            MediaParameterSpec {
                                name: "size".to_string(),
                                label: "Size".to_string(),
                                values: vec!["1024x1024".to_string(), "1536x1024".to_string()],
                                default: "1024x1024".to_string(),
                                request_field: Some("size".to_string()),
                            },
                            MediaParameterSpec {
                                name: "quality".to_string(),
                                label: "Quality".to_string(),
                                values: vec!["auto".to_string(), "high".to_string()],
                                default: "auto".to_string(),
                                request_field: Some("quality".to_string()),
                            },
                            MediaParameterSpec {
                                name: "output_format".to_string(),
                                label: "Output format".to_string(),
                                values: vec!["png".to_string(), "webp".to_string()],
                                default: "png".to_string(),
                                request_field: Some("output_format".to_string()),
                            },
                        ],
                    }],
                }),
            }),
            models: Vec::<ModelDescriptor>::new(),
        });
        registry
    }

    fn chat_router_registry(base_url: String) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderDescriptor {
            id: "openrouter".to_string(),
            display_name: "OpenRouter".to_string(),
            base_url,
            default_api: "openai-completions".to_string(),
            auth_modes: vec![AuthMode::ApiKey],
            headers: IndexMap::new(),
            query_params: IndexMap::new(),
            chat_completions_path: None,
            discovery: None,
            media: Some(ProviderMediaDescriptor {
                image: Some(ImageMediaDescriptor {
                    discovery: None,
                    execution: Some(MediaExecutionDescriptor {
                        adapter: MediaExecutionKind::ChatImageOutput,
                        base_url: None,
                        path: "/chat/completions".to_string(),
                    }),
                    models: Vec::new(),
                }),
            }),
            models: Vec::<ModelDescriptor>::new(),
        });
        registry
    }

    fn discovered_chat_image_cache() -> ExactMediaDiscoveryCache {
        ExactMediaDiscoveryCache::from_inner_for_test(
            crate::runtime::media::resolver::MediaDiscoveryCache {
                image_models: vec![crate::runtime::media::resolver::CachedImageMediaModel {
                    provider_id: "openrouter".to_string(),
                    model: MediaModelDescriptor {
                        id: "openrouter/image-chat".to_string(),
                        display_name: Some("Image Chat".to_string()),
                        execution: None,
                        operations: vec![MediaOperation::Generate],
                        parameters: Vec::new(),
                    },
                    source: "provider_discovery".to_string(),
                }],
            },
            1_000,
        )
    }

    fn auth_store() -> AuthStore {
        let mut auth_store = AuthStore::default();
        auth_store.set_api_key("exact-provider", "sk-test");
        auth_store
    }

    fn openrouter_auth_store() -> AuthStore {
        let mut auth_store = AuthStore::default();
        auth_store.set_api_key("openrouter", "sk-test");
        auth_store
    }

    fn image_generation_tool_definition() -> ToolDefinition {
        ToolDefinition {
            id: "ImageGeneration".to_string(),
            name: "ImageGeneration".to_string(),
            description: "Generate an image".to_string(),
            handler: "runtime:workflow:image_generation".to_string(),
            aliases: Vec::new(),
            handler_args: Vec::new(),
            kind: ToolKind::Custom,
            input_schema: ToolInputSchema::default(),
            metadata: ToolMetadata::default(),
            policy: ToolPolicyHints::default(),
            shared_lib: None,
            enabled_if: None,
            display: ToolDisplayHints::default(),
        }
    }

    fn allow_all_filesystem_policy(root: &Path) -> FilesystemPermissionPolicy {
        FilesystemPermissionPolicy {
            approval: EffectiveApprovalPolicy::Allow,
            sandbox_mode: EffectiveSandboxMode::DangerFullAccess,
            workspace_roots: vec![root.to_path_buf()],
            session_granted: true,
            allow_all_paths: true,
        }
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut buffer = [0_u8; 8192];
        let size = stream.read(&mut buffer).expect("read request");
        String::from_utf8_lossy(&buffer[..size]).to_string()
    }

    fn spawn_image_generation_server() -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let request_text = read_http_request(&mut stream);
            let body = json!({
                "data": [{"b64_json": "aW1hZ2UtYnl0ZXM="}]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("response");
            request_text
        });
        (format!("http://{address}"), handle)
    }

    fn spawn_chat_image_generation_server() -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let request_text = read_http_request(&mut stream);
            let body = json!({
                "choices": [{
                    "message": {
                        "images": [{"b64_json": "aW1hZ2UtYnl0ZXM="}]
                    }
                }]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("response");
            request_text
        });
        (format!("http://{address}"), handle)
    }

    #[path = "image_generation_tool_tests.rs"]
    mod tool_tests;

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
            adapter: Some("images_json".to_string()),
            parameters: BTreeMap::from([("output_format".to_string(), "webp".to_string())]),
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
        assert_eq!(request.parameters["size"], "1024x1024");
        assert_eq!(request.output_path, dir.path().join("out/image.png"));
    }

    #[test]
    fn builds_request_maps_aspect_to_aspect_ratio_parameter() {
        let dir = tempdir().unwrap();
        let settings = puffer_config::ImageMediaConfig {
            provider_id: Some("minimax".to_string()),
            model_id: Some("image-01".to_string()),
            adapter: Some("minimax_image".to_string()),
            parameters: BTreeMap::from([
                ("aspect_ratio".to_string(), "1:1".to_string()),
                ("response_format".to_string(), "base64".to_string()),
            ]),
        };

        let request = build_image_request(
            dir.path(),
            ImageGenerationInput {
                prompt: "make a visual summary".to_string(),
                prompt_reference: None,
                aspect: Some("landscape".to_string()),
                output_path: None,
                purpose: None,
                retry_from_error: None,
            },
            &settings,
        )
        .unwrap();

        assert_eq!(request.parameters["aspect_ratio"], "16:9");
        assert!(!request.parameters.contains_key("size"));
    }

    #[test]
    fn builds_request_preserves_model_specific_size_tokens_for_aspect() {
        let dir = tempdir().unwrap();
        let settings = puffer_config::ImageMediaConfig {
            provider_id: Some("byteplus".to_string()),
            model_id: Some("seedream-4-5-251128".to_string()),
            adapter: Some("images_json".to_string()),
            parameters: BTreeMap::from([
                ("size".to_string(), "2K".to_string()),
                ("output_format".to_string(), "png".to_string()),
            ]),
        };

        let request = build_image_request(
            dir.path(),
            ImageGenerationInput {
                prompt: "make a visual summary".to_string(),
                prompt_reference: None,
                aspect: Some("portrait".to_string()),
                output_path: None,
                purpose: None,
                retry_from_error: None,
            },
            &settings,
        )
        .unwrap();

        assert_eq!(request.parameters["size"], "2K");
    }

    #[test]
    fn builds_request_rejects_aspect_when_selected_model_has_no_aspect_parameter() {
        let dir = tempdir().unwrap();
        let settings = puffer_config::ImageMediaConfig {
            provider_id: Some("openrouter".to_string()),
            model_id: Some("image-chat".to_string()),
            adapter: Some("chat_image_output".to_string()),
            parameters: BTreeMap::new(),
        };

        let error = build_image_request(
            dir.path(),
            ImageGenerationInput {
                prompt: "make a visual summary".to_string(),
                prompt_reference: None,
                aspect: Some("square".to_string()),
                output_path: None,
                purpose: None,
                retry_from_error: None,
            },
            &settings,
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "selected image model does not support ImageGeneration aspect"
        );
    }

    #[test]
    fn builds_request_from_media_settings_instead_of_env_model() {
        let dir = tempdir().unwrap();
        std::env::set_var("PUFFER_IMAGE_MODEL", "legacy-env-model");
        let settings = puffer_config::ImageMediaConfig {
            provider_id: Some("openai".to_string()),
            model_id: Some("configured-image-model".to_string()),
            adapter: Some("images_json".to_string()),
            parameters: BTreeMap::from([
                ("size".to_string(), "1024x1024".to_string()),
                ("quality".to_string(), "high".to_string()),
                ("output_format".to_string(), "webp".to_string()),
            ]),
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
        assert_eq!(request.parameters["quality"], "high");
        assert_eq!(request.output_format, "webp");
        std::env::remove_var("PUFFER_IMAGE_MODEL");
    }

    #[test]
    fn builds_request_for_non_openai_exact_provider() {
        let dir = tempdir().unwrap();
        let settings = puffer_config::ImageMediaConfig {
            provider_id: Some("exact-provider".to_string()),
            model_id: Some("exact-image-model".to_string()),
            adapter: Some("images_json".to_string()),
            parameters: BTreeMap::from([
                ("size".to_string(), "1024x1024".to_string()),
                ("quality".to_string(), "auto".to_string()),
                ("output_format".to_string(), "png".to_string()),
            ]),
        };

        let request = build_image_request(
            dir.path(),
            ImageGenerationInput {
                prompt: "make a visual summary".to_string(),
                prompt_reference: None,
                aspect: None,
                output_path: Some("out/image.png".to_string()),
                purpose: None,
                retry_from_error: None,
            },
            &settings,
        )
        .unwrap();

        assert_eq!(request.provider, "exact-provider");
        assert_eq!(request.model, "exact-image-model");
    }

    #[test]
    fn execute_rejects_missing_provider_model_config() {
        let dir = tempdir().unwrap();
        let registry = registry_with_provider("http://127.0.0.1:9".to_string());
        let auth_store = auth_store();
        let discovery_cache = ExactMediaDiscoveryCache::empty();
        let mut state = test_state(ImageMediaConfig::default(), dir.path());

        let error = execute_image_generation(
            &mut state,
            dir.path(),
            json!({"prompt": "draw a ship"}),
            Some(ImageGenerationMediaContext {
                providers: &registry,
                auth_store: &auth_store,
                discovery_cache: &discovery_cache,
            }),
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "image media provider/model/adapter is not configured"
        );
    }

    #[test]
    fn execute_rejects_stale_exact_model_before_http() {
        let dir = tempdir().unwrap();
        let registry = registry_with_provider("http://127.0.0.1:9".to_string());
        let auth_store = auth_store();
        let discovery_cache = ExactMediaDiscoveryCache::empty();
        let mut state = test_state(
            ImageMediaConfig {
                provider_id: Some("exact-provider".to_string()),
                model_id: Some("stale-image-model".to_string()),
                adapter: Some("images_json".to_string()),
                parameters: BTreeMap::from([
                    ("size".to_string(), "1024x1024".to_string()),
                    ("quality".to_string(), "auto".to_string()),
                    ("output_format".to_string(), "png".to_string()),
                ]),
            },
            dir.path(),
        );

        let error = execute_image_generation(
            &mut state,
            dir.path(),
            json!({"prompt": "draw a ship"}),
            Some(ImageGenerationMediaContext {
                providers: &registry,
                auth_store: &auth_store,
                discovery_cache: &discovery_cache,
            }),
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "selected image model unavailable: exact-provider/stale-image-model via images_json"
        );
    }

    #[test]
    fn execute_uses_descriptor_adapter_and_writes_requested_output_path() {
        let (base_url, server) = spawn_image_generation_server();
        let dir = tempdir().unwrap();
        let registry = registry_with_provider(base_url);
        let auth_store = auth_store();
        let discovery_cache = ExactMediaDiscoveryCache::empty();
        let mut state = test_state(
            ImageMediaConfig {
                provider_id: Some("exact-provider".to_string()),
                model_id: Some("exact-image-model".to_string()),
                adapter: Some("images_json".to_string()),
                parameters: BTreeMap::from([
                    ("size".to_string(), "1024x1024".to_string()),
                    ("quality".to_string(), "auto".to_string()),
                    ("output_format".to_string(), "png".to_string()),
                ]),
            },
            dir.path(),
        );

        let output = execute_image_generation(
            &mut state,
            dir.path(),
            json!({
                "prompt": "draw a ship",
                "outputPath": "requested/ship.png",
                "purpose": "test"
            }),
            Some(ImageGenerationMediaContext {
                providers: &registry,
                auth_store: &auth_store,
                discovery_cache: &discovery_cache,
            }),
        )
        .unwrap();

        let request_text = server.join().expect("server");
        assert!(request_text.starts_with("POST /custom/images HTTP/1.1"));
        assert!(request_text.contains("\"model\":\"exact-image-model\""));
        assert_eq!(
            fs::read(dir.path().join("requested/ship.png")).unwrap(),
            b"image-bytes"
        );
        assert!(dir.path().join(".puffer/media/jobs").is_dir());
        assert!(dir.path().join(".puffer/media/artifact-sidecars").is_dir());

        let parsed: Value = serde_json::from_str(&output).unwrap();
        let expected_path = dir.path().join("requested/ship.png");
        assert_eq!(
            parsed["path"].as_str(),
            Some(expected_path.to_str().unwrap())
        );
        assert_eq!(parsed["provider"], "exact-provider");
        assert_eq!(parsed["model"], "exact-image-model");
        assert_eq!(parsed["status"], "succeeded");
        assert_eq!(parsed["purpose"], "test");
    }
}
