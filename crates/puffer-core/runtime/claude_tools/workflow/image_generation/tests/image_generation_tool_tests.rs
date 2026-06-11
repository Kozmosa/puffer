use super::*;

#[test]
fn execute_uses_discovery_cache_for_chat_image_output_model() {
    let (base_url, server) = spawn_chat_image_generation_server();
    let dir = tempdir().unwrap();
    let registry = chat_router_registry(base_url);
    let auth_store = openrouter_auth_store();
    let discovery_cache = discovered_chat_image_cache();
    let mut state = test_state(
        MediaGenerationConfig {
            provider_id: "openrouter".to_string(),
            logical_model_id: "openrouter/image-chat".to_string(),
            selections: BTreeMap::new(),
        },
        dir.path(),
    );

    let output = execute_image_generation(
        &mut state,
        dir.path(),
        json!({"prompt": "draw a ship", "count": 1}),
        Some(ImageGenerationMediaContext {
            providers: &registry,
            auth_store: &auth_store,
            discovery_cache: &discovery_cache,
        }),
    )
    .unwrap();

    let request_text = server.join().expect("server");
    assert!(request_text.starts_with("POST /chat/completions HTTP/1.1"));
    assert!(request_text.contains("\"model\":\"openrouter/image-chat\""));
    let parsed: Value = serde_json::from_str(&output).unwrap();
    let artifact_path = PathBuf::from(parsed["artifacts"][0]["path"].as_str().unwrap());
    assert_eq!(fs::read(&artifact_path).unwrap(), b"image-bytes");
    assert_eq!(parsed["provider"], "openrouter");
    assert_eq!(parsed["model"], "openrouter/image-chat");
    assert_eq!(parsed["status"], "succeeded");
}

#[test]
fn dispatcher_passes_media_context_to_image_generation_tool() {
    let (base_url, server) = spawn_image_generation_server();
    let dir = tempdir().unwrap();
    let registry = registry_with_provider(base_url);
    let auth_store = auth_store();
    let mut state = test_state(
        MediaGenerationConfig {
            provider_id: "exact-provider".to_string(),
            logical_model_id: "exact-image-model".to_string(),
            selections: BTreeMap::from([
                ("size".to_string(), "1024x1024".to_string()),
                ("quality".to_string(), "auto".to_string()),
                ("output_format".to_string(), "png".to_string()),
            ]),
        },
        dir.path(),
    );
    let definition = image_generation_tool_definition();

    let result = execute_tool(
        &mut state,
        &LoadedResources::default(),
        &registry,
        &auth_store,
        &ToolRegistry::default(),
        &definition,
        dir.path(),
        &allow_all_filesystem_policy(dir.path()),
        json!({"prompt": "draw a routed ship", "count": 1}),
        ProviderToolContext::None,
    )
    .unwrap();

    let request_text = server.join().expect("server");
    assert!(result.success);
    assert!(request_text.starts_with("POST /custom/images HTTP/1.1"));
    assert!(request_text.contains("\"model\":\"exact-image-model\""));
    let parsed: Value = serde_json::from_str(&result.output.stdout).unwrap();
    let artifact_path = PathBuf::from(parsed["artifacts"][0]["path"].as_str().unwrap());
    assert_eq!(fs::read(&artifact_path).unwrap(), b"image-bytes");
}

#[test]
fn image_generation_output_includes_artifacts_array() {
    let output = image_generation_output(&ImageGenerationResult {
        job_id: "job-1".to_string(),
        requested_count: 2,
        artifacts: vec![
            ImageGenerationArtifactResult {
                artifact_id: "artifact-1".to_string(),
                index: 0,
                path: PathBuf::from("/tmp/image-1.png"),
                mime_type: "image/png".to_string(),
                byte_count: 10,
            },
            ImageGenerationArtifactResult {
                artifact_id: "artifact-2".to_string(),
                index: 1,
                path: PathBuf::from("/tmp/image-2.png"),
                mime_type: "image/png".to_string(),
                byte_count: 11,
            },
        ],
        provider: "openai".to_string(),
        model: "gpt-image-1".to_string(),
        status: "succeeded".to_string(),
        parameters: BTreeMap::new(),
        purpose: None,
        retry_from_error: false,
    })
    .unwrap();
    let parsed: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(parsed["jobId"], "job-1");
    assert_eq!(parsed["requestedCount"], 2);
    assert!(parsed.get("artifactId").is_none());
    assert!(parsed.get("path").is_none());
    assert_eq!(parsed["artifacts"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["artifacts"][0]["artifactId"], "artifact-1");
    assert_eq!(parsed["artifacts"][1]["index"], 1);
}
