use super::*;
use indexmap::IndexMap;
use puffer_provider_registry::{
    AuthMode, AuthStore, MediaExecutionDescriptor, MediaExecutionKind, MediaKindDescriptor,
    MediaModelDescriptor, MediaOperation, MediaParameterSpec, MediaParameterWireType,
    ModelDescriptor, ProviderDescriptor, ProviderMediaDescriptor, ProviderRegistry,
};
use puffer_resources::ProviderPack;
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use tempfile::tempdir;

fn minimax_registry(base_url: String) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderDescriptor {
        id: "minimax".to_string(),
        display_name: "MiniMax".to_string(),
        base_url: "https://api.minimax.io/anthropic".to_string(),
        default_api: "anthropic-messages".to_string(),
        auth_modes: vec![AuthMode::ApiKey],
        headers: IndexMap::new(),
        query_params: IndexMap::new(),
        chat_completions_path: None,
        discovery: None,
        media: Some(ProviderMediaDescriptor {
            image: Some(MediaKindDescriptor {
                discovery: None,
                execution: Some(MediaExecutionDescriptor {
                    adapter: MediaExecutionKind::MinimaxImage,
                    base_url: Some(base_url),
                    path: "/v1/image_generation".to_string(),
                    batch: puffer_provider_registry::MediaBatchDescriptor::default(),
                }),
                models: vec![MediaModelDescriptor {
                    id: "image-01".to_string(),
                    display_name: Some("Image 01".to_string()),
                    execution: None,
                    operations: vec![MediaOperation::Generate],
                    parameters: vec![
                        MediaParameterSpec {
                            name: "aspect_ratio".to_string(),
                            label: "Aspect ratio".to_string(),
                            values: vec!["1:1".to_string(), "16:9".to_string()],
                            default: "1:1".to_string(),
                            request_field: Some("aspect_ratio".to_string()),
                            wire_type: MediaParameterWireType::String,
                        },
                        MediaParameterSpec {
                            name: "response_format".to_string(),
                            label: "Response format".to_string(),
                            values: vec!["url".to_string(), "base64".to_string()],
                            default: "base64".to_string(),
                            request_field: Some("response_format".to_string()),
                            wire_type: MediaParameterWireType::String,
                        },
                    ],
                }],
            }),
            video: None,
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
            image: Some(MediaKindDescriptor {
                discovery: None,
                execution: Some(MediaExecutionDescriptor {
                    adapter: MediaExecutionKind::ChatImageOutput,
                    base_url: None,
                    path: "/chat/completions".to_string(),
                    batch: puffer_provider_registry::MediaBatchDescriptor::default(),
                }),
                models: Vec::new(),
            }),
            video: None,
        }),
        models: Vec::<ModelDescriptor>::new(),
    });
    registry
}

fn byteplus_seedream_registry(base_url: String) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderDescriptor {
        id: "byteplus".to_string(),
        display_name: "BytePlus".to_string(),
        base_url,
        default_api: "openai-completions".to_string(),
        auth_modes: vec![AuthMode::ApiKey],
        headers: IndexMap::new(),
        query_params: IndexMap::new(),
        chat_completions_path: None,
        discovery: None,
        media: Some(ProviderMediaDescriptor {
            image: Some(MediaKindDescriptor {
                discovery: None,
                execution: Some(MediaExecutionDescriptor {
                    adapter: MediaExecutionKind::ImagesJson,
                    base_url: None,
                    path: "/images/generations".to_string(),
                    batch: puffer_provider_registry::MediaBatchDescriptor::default(),
                }),
                models: vec![MediaModelDescriptor {
                    id: "seedream-4-5-251128".to_string(),
                    display_name: Some("Seedream 4.5".to_string()),
                    execution: None,
                    operations: vec![MediaOperation::Generate],
                    parameters: vec![
                        MediaParameterSpec {
                            name: "size".to_string(),
                            label: "Size".to_string(),
                            values: vec!["2K".to_string()],
                            default: "2K".to_string(),
                            request_field: Some("size".to_string()),
                            wire_type: MediaParameterWireType::String,
                        },
                        MediaParameterSpec {
                            name: "response_format".to_string(),
                            label: "Response format".to_string(),
                            values: vec!["b64_json".to_string(), "url".to_string()],
                            default: "b64_json".to_string(),
                            request_field: Some("response_format".to_string()),
                            wire_type: MediaParameterWireType::String,
                        },
                    ],
                }],
            }),
            video: None,
        }),
        models: Vec::<ModelDescriptor>::new(),
    });
    registry
}

fn replicate_video_registry() -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderDescriptor {
        id: "replicate".to_string(),
        display_name: "Replicate".to_string(),
        base_url: "https://api.replicate.com".to_string(),
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
                    adapter: MediaExecutionKind::ReplicateVideo,
                    base_url: None,
                    path: "/v1/predictions".to_string(),
                    batch: puffer_provider_registry::MediaBatchDescriptor::default(),
                }),
                models: vec![MediaModelDescriptor {
                    id: "owner/model-version".to_string(),
                    display_name: Some("Video Model".to_string()),
                    execution: None,
                    operations: vec![MediaOperation::Generate],
                    parameters: vec![
                        MediaParameterSpec {
                            name: "aspect_ratio".to_string(),
                            label: "Aspect ratio".to_string(),
                            values: vec!["16:9".to_string(), "9:16".to_string()],
                            default: "16:9".to_string(),
                            request_field: Some("aspect_ratio".to_string()),
                            wire_type: MediaParameterWireType::String,
                        },
                        MediaParameterSpec {
                            name: "duration".to_string(),
                            label: "Duration".to_string(),
                            values: vec!["5".to_string(), "8".to_string()],
                            default: "5".to_string(),
                            request_field: Some("duration".to_string()),
                            wire_type: MediaParameterWireType::String,
                        },
                    ],
                }],
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
    let mut auth = AuthStore::default();
    auth.set_api_key("minimax", "sk-minimax");
    auth
}

fn auth_store_for(provider_id: &str) -> AuthStore {
    let mut auth = AuthStore::default();
    auth.set_api_key(provider_id, "sk-test");
    auth
}

fn bundled_provider(provider_id: &str, yaml: &str) -> ProviderDescriptor {
    let pack: ProviderPack = serde_yaml::from_str(yaml).expect("provider yaml parses");
    assert_eq!(pack.id, provider_id);
    pack.into_descriptor()
}

fn replicate_video_runtime_fixture() -> (
    ProviderRegistry,
    AuthStore,
    ExactMediaDiscoveryCache,
    tempfile::TempDir,
) {
    (
        replicate_video_registry(),
        auth_store_for("replicate"),
        ExactMediaDiscoveryCache::empty(),
        tempdir().expect("tempdir"),
    )
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut buffer = [0_u8; 8192];
    let size = stream.read(&mut buffer).expect("read request");
    String::from_utf8_lossy(&buffer[..size]).to_string()
}

#[test]
fn exact_image_generation_rejects_invalid_count() {
    assert!(validate_image_count(1).is_ok());
    assert!(validate_image_count(4).is_ok());
    assert_eq!(
        validate_image_count(0).unwrap_err().to_string(),
        "image generation count must be between 1 and 4"
    );
    assert_eq!(
        validate_image_count(5).unwrap_err().to_string(),
        "image generation count must be between 1 and 4"
    );
}

#[test]
fn exact_generation_result_returns_artifacts_in_order() {
    let job = MediaJob {
        id: "job-1".to_string(),
        kind: MediaKind::Image,
        provider_id: "openai".to_string(),
        model_id: "gpt-image-1".to_string(),
        adapter: Some("images_json".to_string()),
        prompt: "draw".to_string(),
        parameters: BTreeMap::from([("size".to_string(), "1024x1024".to_string())]),
        status: MediaJobStatus::Succeeded,
        provider_job_id: None,
        remote_status: None,
        remote_get_url: None,
        remote_cancel_url: None,
        artifact_ids: vec!["artifact-1".to_string(), "artifact-2".to_string()],
        requested_count: 2,
        error: None,
        created_at_ms: 1,
        updated_at_ms: 2,
    };
    let artifacts = vec![
        MediaArtifact {
            id: "artifact-1".to_string(),
            job_id: "job-1".to_string(),
            kind: MediaKind::Image,
            path: PathBuf::from("/tmp/image-1.png"),
            mime_type: "image/png".to_string(),
            byte_count: 10,
            metadata: serde_json::json!({"index": 0}),
            preview: None,
            created_at_ms: 1,
        },
        MediaArtifact {
            id: "artifact-2".to_string(),
            job_id: "job-1".to_string(),
            kind: MediaKind::Image,
            path: PathBuf::from("/tmp/image-2.png"),
            mime_type: "image/png".to_string(),
            byte_count: 11,
            metadata: serde_json::json!({"index": 1}),
            preview: None,
            created_at_ms: 1,
        },
    ];

    let result = exact_generation_result(job, artifacts);

    assert_eq!(result.job_id, "job-1");
    assert_eq!(result.requested_count, 2);
    assert_eq!(result.artifacts.len(), 2);
    assert_eq!(result.artifacts[0].artifact_id, "artifact-1");
    assert_eq!(result.artifacts[0].index, 0);
    assert_eq!(result.artifacts[1].artifact_id, "artifact-2");
    assert_eq!(result.artifacts[1].index, 1);
}

#[test]
fn generate_exact_image_dispatches_to_minimax_adapter() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request");
        let request_text = read_http_request(&mut stream);
        let body = json!({
            "data": {"image_base64": ["aW1hZ2UtYnl0ZXM="]},
            "base_resp": {"status_code": 0, "status_msg": "success"}
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
    let registry = minimax_registry(format!("http://{address}"));
    let workspace = tempdir().expect("tempdir");

    let result = generate_exact_image_with_cache(
        &registry,
        &auth_store(),
        workspace.path(),
        ExactImageGenerationRequest {
            provider_id: "minimax".to_string(),
            model_id: "image-01".to_string(),
            adapter: "minimax_image".to_string(),
            prompt: "draw a precise icon".to_string(),
            parameters: BTreeMap::from([
                ("aspect_ratio".to_string(), "16:9".to_string()),
                ("response_format".to_string(), "base64".to_string()),
            ]),
            count: 1,
        },
        &ExactMediaDiscoveryCache::empty(),
    )
    .expect("generation succeeds");

    let request_text = server.join().expect("server");
    assert!(request_text.starts_with("POST /v1/image_generation HTTP/1.1"));
    assert!(request_text.contains("\"aspect_ratio\":\"16:9\""));
    assert_eq!(result.provider_id, "minimax");
    assert_eq!(result.model_id, "image-01");
    assert_eq!(
        std::fs::read(&result.artifacts[0].path).unwrap(),
        b"image-bytes"
    );
}

#[test]
fn generate_exact_image_with_cache_executes_discovered_chat_image_model() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
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
    let registry = chat_router_registry(format!("http://{address}"));
    let workspace = tempdir().expect("tempdir");
    let cache = discovered_chat_image_cache();

    let result = generate_exact_image_with_cache(
        &registry,
        &auth_store_for("openrouter"),
        workspace.path(),
        ExactImageGenerationRequest {
            provider_id: "openrouter".to_string(),
            model_id: "openrouter/image-chat".to_string(),
            adapter: "chat_image_output".to_string(),
            prompt: "draw a precise icon".to_string(),
            parameters: BTreeMap::new(),
            count: 1,
        },
        &cache,
    )
    .expect("generation succeeds");

    let request_text = server.join().expect("server");
    assert!(request_text.starts_with("POST /chat/completions HTTP/1.1"));
    assert!(request_text.contains("\"model\":\"openrouter/image-chat\""));
    assert_eq!(result.provider_id, "openrouter");
    assert_eq!(result.model_id, "openrouter/image-chat");
    assert_eq!(
        std::fs::read(&result.artifacts[0].path).unwrap(),
        b"image-bytes"
    );
}

#[test]
fn generate_exact_image_prunes_stale_undeclared_parameters_before_http() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
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
    let registry = byteplus_seedream_registry(format!("http://{address}"));
    let workspace = tempdir().expect("tempdir");

    generate_exact_image_with_cache(
        &registry,
        &auth_store_for("byteplus"),
        workspace.path(),
        ExactImageGenerationRequest {
            provider_id: "byteplus".to_string(),
            model_id: "seedream-4-5-251128".to_string(),
            adapter: "images_json".to_string(),
            prompt: "draw a precise icon".to_string(),
            parameters: BTreeMap::from([
                ("size".to_string(), "2K".to_string()),
                ("output_format".to_string(), "png".to_string()),
            ]),
            count: 1,
        },
        &ExactMediaDiscoveryCache::empty(),
    )
    .expect("generation succeeds");

    let request_text = server.join().expect("server");
    assert!(request_text.contains("\"size\":\"2K\""));
    assert!(request_text.contains("\"response_format\":\"b64_json\""));
    assert!(!request_text.contains("output_format"));
}

#[test]
fn generate_exact_image_with_cache_rejects_discovered_model_missing_from_cache_before_http() {
    let registry = chat_router_registry("http://127.0.0.1:9".to_string());
    let workspace = tempdir().expect("tempdir");

    let error = generate_exact_image_with_cache(
        &registry,
        &auth_store_for("openrouter"),
        workspace.path(),
        ExactImageGenerationRequest {
            provider_id: "openrouter".to_string(),
            model_id: "openrouter/image-chat".to_string(),
            adapter: "chat_image_output".to_string(),
            prompt: "draw a precise icon".to_string(),
            parameters: BTreeMap::new(),
            count: 1,
        },
        &ExactMediaDiscoveryCache::empty(),
    )
    .expect_err("missing discovery cache should fail");

    assert_eq!(
        error.to_string(),
        "selected image model unavailable: openrouter/openrouter/image-chat via chat_image_output"
    );
}

#[test]
fn list_video_capabilities_exposes_multiple_static_seedance_models() {
    let mut registry = ProviderRegistry::new();
    registry.register_many(vec![
        bundled_provider(
            "byteplus",
            include_str!("../../resources/providers/byteplus.yaml"),
        ),
        bundled_provider(
            "relaydance",
            include_str!("../../resources/providers/relaydance.yaml"),
        ),
    ]);
    let mut auth = AuthStore::default();
    auth.set_api_key("byteplus", "sk-test");
    auth.set_api_key("relaydance", "sk-test");

    let capabilities = list_exact_media_capabilities_with_cache(
        &registry,
        &auth,
        Some("video"),
        &ExactMediaDiscoveryCache::empty(),
    );
    let ids = capabilities
        .iter()
        .map(|capability| capability.model_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        ids,
        std::collections::BTreeSet::from([
            "dreamina-seedance-2-0-260128",
            "dreamina-seedance-2-0-fast-260128",
            "doubao-seedance-2-0-720p",
            "doubao-seedance-2-0-1080p",
            "doubao-seedance-2-0-fast-260128",
            "grok-imagine-video",
            "grok-imagine-video-1.5-preview",
            "happyhorse-1.0-t2v",
            "seedance-1-5-pro-no-audio",
            "seedance-1-5-pro-with-audio",
            "seedance-fast-nsfw",
            "seedance-nsfw",
            "seedance-nsfw-720p",
            "seedance-nsfw-1080p",
        ])
    );
    assert!(capabilities.iter().all(|capability| {
        capability.kind == "video"
            && capability.operation == "generate"
            && capability.status == "available"
            && capability.source == "static"
    }));

    let byteplus_fast = capabilities
        .iter()
        .find(|capability| capability.model_id == "dreamina-seedance-2-0-fast-260128")
        .expect("byteplus fast model");
    let fast_resolution = byteplus_fast
        .parameters
        .iter()
        .find(|parameter| parameter.name == "resolution")
        .expect("fast resolution");
    assert_eq!(
        fast_resolution.values,
        vec!["480p".to_string(), "720p".to_string()]
    );

    let relaydance_1080 = capabilities
        .iter()
        .find(|capability| capability.model_id == "doubao-seedance-2-0-1080p")
        .expect("relaydance 1080p model");
    let relaydance_resolution = relaydance_1080
        .parameters
        .iter()
        .find(|parameter| parameter.name == "resolution")
        .expect("relaydance resolution");
    assert_eq!(relaydance_resolution.values, vec!["1080p".to_string()]);

    let grok_video = capabilities
        .iter()
        .find(|capability| capability.model_id == "grok-imagine-video")
        .expect("grok imagine video model");
    assert!(
        grok_video.parameters.is_empty(),
        "grok imagine video stays prompt-only until RelayDance publishes parameter metadata"
    );
}

#[test]
fn exact_media_generation_rejects_unsupported_video_parameter() {
    let (registry, auth, cache, workspace) = replicate_video_runtime_fixture();
    let request = ExactMediaGenerationRequest {
        kind: "video".to_string(),
        provider_id: "replicate".to_string(),
        model_id: "owner/model-version".to_string(),
        operation: "generate".to_string(),
        adapter: "replicate_video".to_string(),
        prompt: "animate a logo".to_string(),
        parameters: BTreeMap::from([
            ("aspect_ratio".to_string(), "1:1".to_string()),
            ("duration".to_string(), "5".to_string()),
        ]),
        count: 1,
    };

    let error =
        generate_exact_media_with_cache(&registry, &auth, workspace.path(), request, &cache)
            .unwrap_err()
            .to_string();
    assert!(error.contains("video generation parameter unsupported: aspect_ratio=1:1"));
}

#[test]
fn exact_media_generation_rejects_unsupported_adapter_before_http() {
    let (registry, auth, cache, workspace) = replicate_video_runtime_fixture();
    let request = ExactMediaGenerationRequest {
        kind: "video".to_string(),
        provider_id: "replicate".to_string(),
        model_id: "owner/model-version".to_string(),
        operation: "generate".to_string(),
        adapter: "images_json".to_string(),
        prompt: "animate a logo".to_string(),
        parameters: BTreeMap::from([
            ("aspect_ratio".to_string(), "16:9".to_string()),
            ("duration".to_string(), "5".to_string()),
        ]),
        count: 1,
    };

    let error =
        generate_exact_media_with_cache(&registry, &auth, workspace.path(), request, &cache)
            .unwrap_err()
            .to_string();
    assert!(error.contains("selected video model unavailable"));
}

#[test]
fn exact_media_discovery_cache_uses_ttl_boundary() {
    let cache = ExactMediaDiscoveryCache::from_inner_for_test(
        crate::runtime::media::resolver::MediaDiscoveryCache::default(),
        1_000,
    );

    assert!(cache.is_fresh_at(1_000 + MEDIA_DISCOVERY_TTL_MS - 1));
    assert!(!cache.is_fresh_at(1_000 + MEDIA_DISCOVERY_TTL_MS + 1));
}
