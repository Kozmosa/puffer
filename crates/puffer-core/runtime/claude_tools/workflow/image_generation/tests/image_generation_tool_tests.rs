use super::*;

#[test]
fn execute_uses_discovery_cache_for_chat_image_output_model() {
    let (base_url, server) = spawn_chat_image_generation_server();
    let dir = tempdir().unwrap();
    let registry = chat_router_registry(base_url);
    let auth_store = openrouter_auth_store();
    let discovery_cache = discovered_chat_image_cache();
    let mut state = test_state(
        ImageMediaConfig {
            provider_id: Some("openrouter".to_string()),
            model_id: Some("openrouter/image-chat".to_string()),
            adapter: Some("chat_image_output".to_string()),
            parameters: BTreeMap::new(),
        },
        dir.path(),
    );

    let output = execute_image_generation(
        &mut state,
        dir.path(),
        json!({
            "prompt": "draw a ship",
            "outputPath": "requested/ship.png"
        }),
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
    assert_eq!(
        fs::read(
            dir.path()
                .join(".puffer/workflows/images/requested/ship.png")
        )
        .unwrap(),
        b"image-bytes"
    );
    let parsed: Value = serde_json::from_str(&output).unwrap();
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
        json!({
            "prompt": "draw a routed ship",
            "outputPath": "requested/routed.png"
        }),
        ProviderToolContext::None,
    )
    .unwrap();

    let request_text = server.join().expect("server");
    assert!(result.success);
    assert!(request_text.starts_with("POST /custom/images HTTP/1.1"));
    assert!(request_text.contains("\"model\":\"exact-image-model\""));
    assert_eq!(
        fs::read(
            dir.path()
                .join(".puffer/workflows/images/requested/routed.png")
        )
        .unwrap(),
        b"image-bytes"
    );
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
        parameters: BTreeMap::from([("size".to_string(), "1024x1024".to_string())]),
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
