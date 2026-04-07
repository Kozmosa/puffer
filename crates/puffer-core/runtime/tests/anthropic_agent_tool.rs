use super::*;

#[test]
fn execute_anthropic_tool_calls_runs_agent_runtime_tool() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured = Arc::clone(&requests);
    let server = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buffer = [0_u8; 4096];
            let size = stream.read(&mut buffer).unwrap();
            captured
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buffer[..size]).to_string());
            let body = json!({
                "content": [
                    {
                        "type": "text",
                        "text": "nested ok"
                    }
                ]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });

    let mut local_provider = provider();
    local_provider.id = "local-anthropic".to_string();
    local_provider.base_url = format!("http://{address}");
    local_provider.auth_modes.clear();
    local_provider.models[0].provider = "local-anthropic".to_string();
    let mut providers = ProviderRegistry::new();
    providers.register(local_provider);

    let resources = LoadedResources {
        tools: vec![loaded_tool("Agent", "Delegate work", "runtime:agent")],
        agents: vec![loaded_agent(
            "Explore",
            "Explore code",
            "You are an explorer.",
            &["read_file"],
        )],
        ..LoadedResources::default()
    };
    let registry = ToolRegistry::from_resources(&resources);
    let response = json!({
        "content": [
            {
                "type": "tool_use",
                "id": "toolu_1",
                "name": "Agent",
                "input": {
                    "description": "Nested task",
                    "prompt": "Inspect the workspace",
                    "subagent_type": "Explore"
                }
            }
        ]
    });
    let mut state = state();
    state.current_provider = Some("local-anthropic".to_string());
    state.current_model = Some("local-anthropic/claude-sonnet-4-5".to_string());
    let request_config = test_anthropic_request_config();

    let result = execute_anthropic_tool_calls(
        &mut state,
        &resources,
        &providers,
        &mut AuthStore::default(),
        &response,
        &registry,
        std::env::current_dir().unwrap().as_path(),
        &request_config,
        "claude-sonnet-4-5",
    )
    .unwrap();

    assert_eq!(result.invocations.len(), 1);
    assert_eq!(result.invocations[0].tool_id, "Agent");
    let payload: Value = serde_json::from_str(&result.invocations[0].output).unwrap();
    assert_eq!(payload["status"], "completed");
    assert_eq!(payload["agentType"], "Explore");
    assert_eq!(payload["result"], "nested ok");

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("\"name\":\"read_file\""));
    assert!(!requests[0].contains("\"name\":\"write_file\""));
    server.join().unwrap();
}
