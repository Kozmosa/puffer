use super::*;
use puffer_config::PufferConfig;
use puffer_provider_registry::{AuthMode, AuthStore, ProviderDescriptor, ProviderRegistry};
use puffer_resources::{AgentSpec, LoadedItem, LoadedResources, SourceInfo, SourceKind};
use puffer_session_store::SessionMetadata;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::thread;
use uuid::Uuid;

fn provider() -> ProviderDescriptor {
    ProviderDescriptor {
        id: "anthropic".to_string(),
        display_name: "Anthropic".to_string(),
        base_url: "https://api.anthropic.com".to_string(),
        default_api: "anthropic-messages".to_string(),
        auth_modes: vec![AuthMode::ApiKey],
        headers: Default::default(),
        query_params: Default::default(),
        discovery: None,
        models: vec![puffer_provider_registry::ModelDescriptor {
            id: "claude-sonnet-4-5".to_string(),
            display_name: "Claude Sonnet 4.5".to_string(),
            provider: "anthropic".to_string(),
            api: "anthropic-messages".to_string(),
            context_window: 200_000,
            max_output_tokens: 8_192,
            supports_reasoning: true,
        }],
    }
}

fn loaded_agent(
    id: &str,
    description: &str,
    prompt: &str,
    tools: &[&str],
) -> LoadedItem<AgentSpec> {
    LoadedItem {
        value: AgentSpec {
            id: id.to_string(),
            description: description.to_string(),
            prompt: prompt.to_string(),
            tools: tools.iter().map(|tool| tool.to_string()).collect(),
            model: None,
        },
        source_info: SourceInfo {
            path: format!("{id}.yaml").into(),
            kind: SourceKind::Builtin,
        },
    }
}

#[test]
fn execute_agent_tool_background_returns_async_payload_and_output_file() {
    let temp = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let body = json!({
                "content": [
                    {
                        "type": "text",
                        "text": "background ok"
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

    let mut provider = provider();
    provider.id = "local-anthropic".to_string();
    provider.base_url = format!("http://{address}");
    provider.auth_modes.clear();
    provider.models[0].provider = "local-anthropic".to_string();

    let mut providers = ProviderRegistry::new();
    providers.register(provider);
    let session = SessionMetadata {
        id: Uuid::new_v4(),
        display_name: None,
        cwd: temp.path().to_path_buf(),
        created_at_ms: 0,
        updated_at_ms: 0,
        parent_session_id: None,
        slug: None,
        tags: Vec::new(),
        note: None,
    };
    let mut state = AppState::new(PufferConfig::default(), temp.path().to_path_buf(), session);
    state.current_provider = Some("local-anthropic".to_string());
    state.current_model = Some("local-anthropic/claude-sonnet-4-5".to_string());

    let resources = LoadedResources {
        agents: vec![loaded_agent(
            "general-purpose",
            "Default agent",
            "You are a coding subagent.",
            &["read_file"],
        )],
        ..LoadedResources::default()
    };
    let output = super::agents::execute_agent_tool(
        &state,
        &resources,
        &providers,
        &mut AuthStore::default(),
        temp.path(),
        json!({
            "description": "Background request",
            "prompt": "Do the thing",
            "run_in_background": true
        }),
    )
    .unwrap();
    let payload: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(payload["status"], "async_launched");
    let output_file = payload["outputFile"].as_str().unwrap();
    assert!(Path::new(output_file).exists());

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let raw = std::fs::read_to_string(output_file).unwrap();
        let written: Value = serde_json::from_str(&raw).unwrap();
        if written["status"] == "completed" {
            assert_eq!(written["result"], "background ok");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "background agent did not finish in time"
        );
        thread::sleep(std::time::Duration::from_millis(20));
    }
    server.join().unwrap();
}
