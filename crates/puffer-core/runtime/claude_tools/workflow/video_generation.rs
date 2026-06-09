use crate::AppState;
use crate::{
    generate_exact_media_with_cache, ExactMediaDiscoveryCache, ExactMediaGenerationRequest,
    ExactMediaGenerationResult,
};
use anyhow::{bail, Context, Result};
use puffer_config::MediaGenerationConfig;
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path};

const MAX_PROMPT_CHARS: usize = 20_000;

/// Carries exact media runtime context into the VideoGeneration workflow tool.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VideoGenerationMediaContext<'a> {
    pub(crate) providers: &'a ProviderRegistry,
    pub(crate) auth_store: &'a AuthStore,
    pub(crate) discovery_cache: &'a ExactMediaDiscoveryCache,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VideoGenerationInput {
    prompt: String,
    #[serde(default)]
    parameters: BTreeMap<String, String>,
    #[serde(default)]
    purpose: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct VideoRequest {
    provider: String,
    model: String,
    operation: String,
    adapter: String,
    prompt: String,
    parameters: BTreeMap<String, String>,
    purpose: Option<String>,
}

/// Builds a text-to-video request from tool input and executes the media runtime.
pub fn execute_video_generation(
    state: &mut AppState,
    cwd: &Path,
    input: Value,
    media_context: Option<VideoGenerationMediaContext<'_>>,
) -> Result<String> {
    let parsed: VideoGenerationInput =
        serde_json::from_value(input).context("invalid VideoGeneration input")?;
    let settings = state
        .config
        .media
        .video
        .as_ref()
        .context("video media provider/model/adapter is not configured")?;
    let request = build_video_request(cwd, parsed, settings)?;
    let media_context = media_context.context("VideoGeneration media runtime is not configured")?;
    let generated = generate_exact_media_with_cache(
        media_context.providers,
        media_context.auth_store,
        cwd,
        exact_media_request(&request),
        media_context.discovery_cache,
    )?;
    video_generation_output(&generated, &request.parameters, request.purpose.as_deref())
}

fn build_video_request(
    cwd: &Path,
    input: VideoGenerationInput,
    settings: &MediaGenerationConfig,
) -> Result<VideoRequest> {
    let prompt = prompt_text(cwd, &input.prompt)?;
    let (provider, model, operation, adapter) = required_video_selection(settings)?;
    let mut parameters = settings.parameters.clone();
    parameters.extend(input.parameters);
    Ok(VideoRequest {
        provider,
        model,
        operation,
        adapter,
        prompt,
        parameters,
        purpose: input.purpose,
    })
}

fn exact_media_request(request: &VideoRequest) -> ExactMediaGenerationRequest {
    ExactMediaGenerationRequest {
        kind: "video".to_string(),
        provider_id: request.provider.clone(),
        model_id: request.model.clone(),
        operation: request.operation.clone(),
        adapter: request.adapter.clone(),
        prompt: request.prompt.clone(),
        parameters: request.parameters.clone(),
        count: 1,
    }
}

fn video_generation_output(
    result: &ExactMediaGenerationResult,
    parameters: &BTreeMap<String, String>,
    purpose: Option<&str>,
) -> Result<String> {
    let artifacts = result
        .artifacts
        .iter()
        .map(|artifact| {
            let mut value = json!({
                "artifactId": artifact.artifact_id,
                "index": artifact.index,
                "path": artifact.path,
                "mimeType": artifact.mime_type,
                "size": artifact.byte_count
            });
            if let Some(remote_source_url) = &artifact.remote_source_url {
                value["remoteSourceUrl"] = json!(remote_source_url);
            }
            value
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_string_pretty(&json!({
        "jobId": result.job_id,
        "kind": result.kind,
        "requestedCount": result.requested_count,
        "artifacts": artifacts,
        "provider": result.provider_id,
        "model": result.model_id,
        "status": result.status,
        "parameters": parameters,
        "purpose": purpose
    }))?)
}

fn required_video_selection(
    settings: &MediaGenerationConfig,
) -> Result<(String, String, String, String)> {
    let provider = settings.provider_id.trim();
    let model = settings.model_id.trim();
    let operation = settings.operation.trim();
    let adapter = settings.adapter.trim();
    if provider.is_empty() || model.is_empty() || adapter.is_empty() {
        bail!("video media provider/model/adapter is not configured");
    }
    if operation.is_empty() {
        bail!("video media operation is not configured");
    }
    Ok((
        provider.to_string(),
        model.to_string(),
        operation.to_string(),
        adapter.to_string(),
    ))
}

fn prompt_text(cwd: &Path, value: &str) -> Result<String> {
    let text = value.trim();
    if text.is_empty() {
        bail!("VideoGeneration prompt is required");
    }
    let candidate = cwd.join(text);
    let prompt = if safe_relative_path(text) && candidate.is_file() {
        fs::read_to_string(&candidate)
            .with_context(|| format!("read VideoGeneration `prompt` {}", candidate.display()))?
    } else {
        text.to_string()
    };
    let prompt = prompt.trim();
    if prompt.is_empty() {
        bail!("VideoGeneration prompt is empty");
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        bail!("VideoGeneration prompt exceeds {MAX_PROMPT_CHARS} characters");
    }
    Ok(prompt.to_string())
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
    use crate::AppState;
    use indexmap::IndexMap;
    use puffer_config::MediaGenerationConfig;
    use puffer_provider_registry::{
        AuthMode, AuthStore, MediaExecutionDescriptor, MediaExecutionKind, MediaKindDescriptor,
        MediaModelDescriptor, MediaOperation, MediaParameterSpec, ModelDescriptor,
        ProviderDescriptor, ProviderMediaDescriptor, ProviderRegistry,
    };
    use puffer_resources::LoadedResources;
    use puffer_session_store::SessionMetadata;
    use puffer_tools::{
        ToolDefinition, ToolDisplayHints, ToolInputSchema, ToolKind, ToolMetadata, ToolPolicyHints,
        ToolRegistry,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::thread;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn video_settings() -> MediaGenerationConfig {
        MediaGenerationConfig {
            provider_id: "relaydance".to_string(),
            model_id: "doubao-seedance-2-0-720p".to_string(),
            operation: "generate".to_string(),
            adapter: "relaydance_video".to_string(),
            parameters: BTreeMap::from([
                ("duration".to_string(), "5".to_string()),
                ("ratio".to_string(), "16:9".to_string()),
                ("resolution".to_string(), "720p".to_string()),
            ]),
        }
    }

    fn test_state(settings: Option<MediaGenerationConfig>, cwd: &Path) -> AppState {
        let mut config = puffer_config::PufferConfig::default();
        config.media.video = settings;
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

    fn video_registry(base_url: String) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderDescriptor {
            id: "relaydance".to_string(),
            display_name: "Relaydance".to_string(),
            base_url,
            default_api: "openai-completions".to_string(),
            auth_modes: vec![AuthMode::ApiKey],
            headers: IndexMap::new(),
            query_params: IndexMap::new(),
            chat_completions_path: None,
            discovery: None,
            media: Some(ProviderMediaDescriptor {
                image: None,
                video: Some(MediaKindDescriptor {
                    discovery: None,
                    execution: Some(MediaExecutionDescriptor {
                        adapter: MediaExecutionKind::RelaydanceVideo,
                        base_url: None,
                        path: "/v1/video/generations".to_string(),
                        batch: puffer_provider_registry::MediaBatchDescriptor::default(),
                    }),
                    models: vec![MediaModelDescriptor {
                        id: "doubao-seedance-2-0-720p".to_string(),
                        display_name: Some("Seedance 2.0".to_string()),
                        execution: None,
                        operations: vec![MediaOperation::Generate],
                        parameters: vec![
                            MediaParameterSpec {
                                name: "duration".to_string(),
                                label: "Duration".to_string(),
                                values: vec![
                                    "4".to_string(),
                                    "5".to_string(),
                                    "6".to_string(),
                                    "7".to_string(),
                                    "8".to_string(),
                                    "9".to_string(),
                                    "10".to_string(),
                                    "11".to_string(),
                                    "12".to_string(),
                                    "13".to_string(),
                                    "14".to_string(),
                                    "15".to_string(),
                                ],
                                default: "5".to_string(),
                                request_field: Some("seconds".to_string()),
                            },
                            MediaParameterSpec {
                                name: "resolution".to_string(),
                                label: "Resolution".to_string(),
                                values: vec![
                                    "480p".to_string(),
                                    "720p".to_string(),
                                    "1080p".to_string(),
                                ],
                                default: "720p".to_string(),
                                request_field: Some("metadata.resolution".to_string()),
                            },
                            MediaParameterSpec {
                                name: "ratio".to_string(),
                                label: "Aspect ratio".to_string(),
                                values: vec![
                                    "16:9".to_string(),
                                    "4:3".to_string(),
                                    "1:1".to_string(),
                                    "3:4".to_string(),
                                    "9:16".to_string(),
                                    "21:9".to_string(),
                                    "adaptive".to_string(),
                                ],
                                default: "16:9".to_string(),
                                request_field: Some("metadata.ratio".to_string()),
                            },
                        ],
                    }],
                }),
            }),
            models: Vec::<ModelDescriptor>::new(),
        });
        registry
    }

    fn auth_store() -> AuthStore {
        let mut auth_store = AuthStore::default();
        auth_store.set_api_key("relaydance", "sk-test");
        auth_store
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let size = stream.read(&mut buffer).expect("read request");
            if size == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..size]);
            let Some(header_end) = header_end(&request) else {
                continue;
            };
            let content_length = content_length(&request[..header_end]).unwrap_or(0);
            let expected_size = header_end + b"\r\n\r\n".len() + content_length;
            while request.len() < expected_size {
                let size = stream.read(&mut buffer).expect("read request body");
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..size]);
            }
            break;
        }
        String::from_utf8_lossy(&request).to_string()
    }

    fn header_end(request: &[u8]) -> Option<usize> {
        request
            .windows(b"\r\n\r\n".len())
            .position(|window| window == b"\r\n\r\n")
    }

    fn content_length(headers: &[u8]) -> Option<usize> {
        let headers = String::from_utf8_lossy(headers);
        headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
    }

    fn http_json_response(body: serde_json::Value) -> String {
        let body = body.to_string();
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    fn http_video_response(bytes: &[u8]) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: video/mp4\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            bytes.len(),
            String::from_utf8_lossy(bytes)
        )
    }

    fn spawn_relaydance_video_server() -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let base_url = format!("http://{address}");
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for index in 0..3 {
                let (mut stream, _) = listener.accept().expect("request");
                let request_text = read_http_request(&mut stream);
                requests.push(request_text);
                let response = match index {
                    0 => http_json_response(json!({
                        "id": "task-1",
                        "status": "queued"
                    })),
                    1 => http_json_response(json!({
                        "id": "task-1",
                        "status": "completed",
                        "metadata": { "url": format!("http://{address}/generated.mp4") }
                    })),
                    _ => http_video_response(b"mp4-bytes"),
                };
                stream.write_all(response.as_bytes()).expect("response");
            }
            requests
        });
        (base_url, handle)
    }

    fn video_generation_tool_definition() -> ToolDefinition {
        ToolDefinition {
            id: "VideoGeneration".to_string(),
            name: "VideoGeneration".to_string(),
            description: "Generate a video".to_string(),
            handler: "runtime:workflow:video_generation".to_string(),
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

    #[test]
    fn execute_rejects_missing_video_provider_model_config() {
        let dir = tempdir().unwrap();
        let registry = ProviderRegistry::new();
        let auth_store = AuthStore::default();
        let discovery_cache = crate::ExactMediaDiscoveryCache::empty();
        let mut state = test_state(None, dir.path());

        let error = execute_video_generation(
            &mut state,
            dir.path(),
            json!({"prompt": "make a ship launch video"}),
            Some(VideoGenerationMediaContext {
                providers: &registry,
                auth_store: &auth_store,
                discovery_cache: &discovery_cache,
            }),
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "video media provider/model/adapter is not configured"
        );
    }

    #[test]
    fn build_request_rejects_empty_prompt() {
        let dir = tempdir().unwrap();

        let error = build_video_request(
            dir.path(),
            VideoGenerationInput {
                prompt: "  ".to_string(),
                parameters: BTreeMap::new(),
                purpose: None,
            },
            &video_settings(),
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "VideoGeneration prompt is required");
    }

    #[test]
    fn rejects_unknown_video_generation_fields() {
        let error = serde_json::from_value::<VideoGenerationInput>(json!({
            "prompt": "make a ship launch video",
            "referenceImage": "frame.png"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("referenceImage"));
    }

    #[test]
    fn build_request_merges_saved_parameters_and_tool_overrides() {
        let dir = tempdir().unwrap();

        let request = build_video_request(
            dir.path(),
            VideoGenerationInput {
                prompt: "make a ship launch video".to_string(),
                parameters: BTreeMap::from([
                    ("ratio".to_string(), "9:16".to_string()),
                    ("resolution".to_string(), "1080p".to_string()),
                ]),
                purpose: Some("short launch clip".to_string()),
            },
            &video_settings(),
        )
        .unwrap();

        assert_eq!(request.provider, "relaydance");
        assert_eq!(request.model, "doubao-seedance-2-0-720p");
        assert_eq!(request.adapter, "relaydance_video");
        assert_eq!(request.parameters["duration"], "5");
        assert_eq!(request.parameters["ratio"], "9:16");
        assert_eq!(request.parameters["resolution"], "1080p");
        assert_eq!(request.purpose.as_deref(), Some("short launch clip"));
    }

    #[test]
    fn execute_uses_exact_video_generation_and_returns_artifacts() {
        let (base_url, server) = spawn_relaydance_video_server();
        let dir = tempdir().unwrap();
        let registry = video_registry(base_url);
        let auth_store = auth_store();
        let discovery_cache = crate::ExactMediaDiscoveryCache::empty();
        let mut state = test_state(Some(video_settings()), dir.path());

        let output = execute_video_generation(
            &mut state,
            dir.path(),
            json!({
                "prompt": "make a ship launch video",
                "parameters": { "ratio": "9:16" },
                "purpose": "short launch clip"
            }),
            Some(VideoGenerationMediaContext {
                providers: &registry,
                auth_store: &auth_store,
                discovery_cache: &discovery_cache,
            }),
        )
        .unwrap();

        let requests = server.join().expect("server");
        assert!(requests[0].starts_with("POST /v1/video/generations HTTP/1.1"));
        assert!(requests[0].contains("\"model\":\"doubao-seedance-2-0-720p\""));
        assert!(requests[0].contains("\"prompt\":\"make a ship launch video\""));
        assert!(requests[0].contains("\"seconds\":\"5\""));
        assert!(requests[0].contains("\"ratio\":\"9:16\""));
        assert!(requests[1].starts_with("GET /v1/video/generations/task-1 HTTP/1.1"));
        assert!(requests[2].starts_with("GET /generated.mp4 HTTP/1.1"));

        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["kind"], "video");
        assert_eq!(parsed["requestedCount"], 1);
        assert_eq!(parsed["provider"], "relaydance");
        assert_eq!(parsed["model"], "doubao-seedance-2-0-720p");
        assert_eq!(parsed["status"], "succeeded");
        assert_eq!(parsed["purpose"], "short launch clip");
        assert_eq!(parsed["parameters"]["duration"], "5");
        assert_eq!(parsed["parameters"]["resolution"], "720p");
        assert_eq!(parsed["parameters"]["ratio"], "9:16");
        let artifacts = parsed["artifacts"].as_array().unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0]["index"], 0);
        assert_eq!(artifacts[0]["mimeType"], "video/mp4");
        assert_eq!(artifacts[0]["size"], 9);
        let artifact_path = std::path::PathBuf::from(artifacts[0]["path"].as_str().unwrap());
        assert_eq!(std::fs::read(&artifact_path).unwrap(), b"mp4-bytes");
        assert!(artifact_path.starts_with(dir.path().join(".puffer/media/videos")));
    }

    #[test]
    fn dispatcher_passes_media_context_to_video_generation_tool() {
        let (base_url, server) = spawn_relaydance_video_server();
        let dir = tempdir().unwrap();
        let registry = video_registry(base_url);
        let auth_store = auth_store();
        let mut state = test_state(Some(video_settings()), dir.path());
        let definition = video_generation_tool_definition();

        let result = execute_tool(
            &mut state,
            &LoadedResources::default(),
            &registry,
            &auth_store,
            &ToolRegistry::default(),
            &definition,
            dir.path(),
            &allow_all_filesystem_policy(dir.path()),
            json!({
                "prompt": "make a routed launch video",
                "parameters": { "ratio": "1:1" }
            }),
            ProviderToolContext::None,
        )
        .unwrap();

        let requests = server.join().expect("server");
        assert!(result.success);
        assert!(requests[0].starts_with("POST /v1/video/generations HTTP/1.1"));
        assert!(requests[0].contains("\"model\":\"doubao-seedance-2-0-720p\""));
        assert!(requests[0].contains("\"ratio\":\"1:1\""));
        let parsed: serde_json::Value = serde_json::from_str(&result.output.stdout).unwrap();
        assert_eq!(parsed["kind"], "video");
        assert_eq!(parsed["artifacts"].as_array().unwrap().len(), 1);
    }
}
