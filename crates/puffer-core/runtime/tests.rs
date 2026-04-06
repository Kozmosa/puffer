use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use puffer_config::PufferConfig;
use puffer_provider_openai::OpenAIResponseToolCall;
use puffer_provider_registry::{AuthMode, ProviderDescriptor, StoredCredential};
use puffer_resources::{LoadedItem, LoadedResources, SourceInfo, SourceKind, ToolSpec};
use puffer_session_store::SessionMetadata;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, OnceLock};
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
            max_output_tokens: 8192,
            supports_reasoning: true,
        }],
    }
}

fn state() -> AppState {
    AppState::new(
        PufferConfig::default(),
        std::env::current_dir().unwrap(),
        SessionMetadata {
            id: Uuid::nil(),
            display_name: None,
            cwd: std::env::current_dir().unwrap(),
            created_at_ms: 0,
            updated_at_ms: 0,
            parent_session_id: None,
            slug: None,
            tags: Vec::new(),
            note: None,
        },
    )
}

fn loaded_tool(id: &str, description: &str, handler: &str) -> LoadedItem<ToolSpec> {
    LoadedItem {
        value: ToolSpec {
            id: id.to_string(),
            name: id.to_string(),
            description: description.to_string(),
            handler: handler.to_string(),
            handler_args: Vec::new(),
            approval_policy: None,
            sandbox_policy: None,
            shared_lib: None,
            enabled_if: None,
            input_schema: None,
            metadata: Default::default(),
            display: Default::default(),
        },
        source_info: SourceInfo {
            path: format!("{id}.yaml").into(),
            kind: SourceKind::Builtin,
        },
    }
}

fn openai_provider(base_url: String) -> ProviderDescriptor {
    ProviderDescriptor {
        id: "openai".to_string(),
        display_name: "OpenAI".to_string(),
        base_url,
        default_api: "openai-responses".to_string(),
        auth_modes: vec![AuthMode::ApiKey, AuthMode::OAuth],
        headers: Default::default(),
        query_params: Default::default(),
        discovery: None,
        models: vec![puffer_provider_registry::ModelDescriptor {
            id: "gpt-5".to_string(),
            display_name: "GPT-5".to_string(),
            provider: "openai".to_string(),
            api: "openai-responses".to_string(),
            context_window: 272_000,
            max_output_tokens: 16_384,
            supports_reasoning: true,
        }],
    }
}

fn fake_jwt(payload: Value) -> String {
    format!("header.{}.sig", URL_SAFE_NO_PAD.encode(payload.to_string()))
}

fn refresh_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn anthropic_tool_schema_lists_expected_fields() {
    let schema = anthropic_tool_schema("write_file");
    let required = schema.get("required").and_then(Value::as_array).unwrap();
    assert!(required.iter().any(|value| value == "path"));
    assert!(required.iter().any(|value| value == "contents"));
}

#[test]
fn resolve_selection_uses_first_provider_model_when_unset() {
    let mut registry = ProviderRegistry::new();
    registry.register(provider());
    let state = state();
    let (provider, model_id) = resolve_provider_and_model(&state, &registry).unwrap();
    assert_eq!(provider.id, "anthropic");
    assert_eq!(model_id, "claude-sonnet-4-5");
}

#[test]
fn resolve_model_api_supports_custom_provider_families() {
    let mut descriptor = provider();
    descriptor.id = "custom-openai".to_string();
    descriptor.display_name = "Custom OpenAI".to_string();
    descriptor.base_url = "https://example.invalid".to_string();
    descriptor.default_api = "openai-responses".to_string();
    descriptor.models[0].provider = "custom-openai".to_string();
    descriptor.models[0].api = "openai-responses".to_string();
    let mut registry = ProviderRegistry::new();
    registry.register(descriptor);
    let mut state = state();
    state.current_provider = Some("custom-openai".to_string());
    let (provider, model_id) = resolve_provider_and_model(&state, &registry).unwrap();
    assert_eq!(
        resolve_model_api(&state, &registry, provider, &model_id),
        "openai-responses"
    );
}

#[test]
fn execute_user_prompt_accepts_openai_family_aliases() {
    let mut descriptor = provider();
    descriptor.id = "azure-openai".to_string();
    descriptor.display_name = "Azure OpenAI".to_string();
    descriptor.base_url = "https://example.invalid".to_string();
    descriptor.default_api = "azure-openai-responses".to_string();
    descriptor.models[0].provider = "azure-openai".to_string();
    descriptor.models[0].api = "azure-openai-responses".to_string();
    let mut registry = ProviderRegistry::new();
    registry.register(descriptor);
    let mut azure_state = state();
    azure_state.current_provider = Some("azure-openai".to_string());
    azure_state.current_model = Some("azure-openai/claude-sonnet-4-5".to_string());
    let mut auth = AuthStore::default();
    auth.set_api_key("azure-openai", "sk-test");
    let error = execute_user_prompt(
        &azure_state,
        &LoadedResources::default(),
        &registry,
        &mut auth,
        "hello",
    )
    .unwrap_err();
    assert!(!error
        .to_string()
        .contains("provider azure-openai with api azure-openai-responses is not executable yet"));

    let mut descriptor = provider();
    descriptor.id = "openrouter".to_string();
    descriptor.display_name = "OpenRouter".to_string();
    descriptor.base_url = "https://example.invalid".to_string();
    descriptor.default_api = "openai-completions".to_string();
    descriptor.models[0].provider = "openrouter".to_string();
    descriptor.models[0].api = "openai-completions".to_string();
    let mut registry = ProviderRegistry::new();
    registry.register(descriptor);
    let mut openrouter_state = state();
    openrouter_state.current_provider = Some("openrouter".to_string());
    openrouter_state.current_model = Some("openrouter/claude-sonnet-4-5".to_string());
    let mut auth = AuthStore::default();
    auth.set_api_key("openrouter", "sk-test");
    let error = execute_user_prompt(
        &openrouter_state,
        &LoadedResources::default(),
        &registry,
        &mut auth,
        "hello",
    )
    .unwrap_err();
    assert!(!error
        .to_string()
        .contains("provider openrouter with api openai-completions is not executable yet"));

    let mut descriptor = provider();
    descriptor.id = "mistral".to_string();
    descriptor.display_name = "Mistral".to_string();
    descriptor.base_url = "https://example.invalid".to_string();
    descriptor.default_api = "mistral-conversations".to_string();
    descriptor.models[0].provider = "mistral".to_string();
    descriptor.models[0].api = "mistral-conversations".to_string();
    let mut registry = ProviderRegistry::new();
    registry.register(descriptor);
    let mut mistral_state = state();
    mistral_state.current_provider = Some("mistral".to_string());
    mistral_state.current_model = Some("mistral/claude-sonnet-4-5".to_string());
    let mut auth = AuthStore::default();
    auth.set_api_key("mistral", "sk-test");
    let error = execute_user_prompt(
        &mistral_state,
        &LoadedResources::default(),
        &registry,
        &mut auth,
        "hello",
    )
    .unwrap_err();
    assert!(!error
        .to_string()
        .contains("provider mistral with api mistral-conversations is not executable yet"));
}

#[test]
fn execute_user_prompt_allows_no_auth_providers() {
    let mut descriptor = provider();
    descriptor.id = "ollama".to_string();
    descriptor.display_name = "Ollama".to_string();
    descriptor.base_url = "http://127.0.0.1:11434".to_string();
    descriptor.default_api = "openai-completions".to_string();
    descriptor.auth_modes.clear();
    descriptor.models[0].provider = "ollama".to_string();
    descriptor.models[0].api = "openai-completions".to_string();
    let mut registry = ProviderRegistry::new();
    registry.register(descriptor);
    let mut state = state();
    state.current_provider = Some("ollama".to_string());
    state.current_model = Some("ollama/claude-sonnet-4-5".to_string());
    let error = execute_user_prompt(
        &state,
        &LoadedResources::default(),
        &registry,
        &mut AuthStore::default(),
        "hello",
    )
    .unwrap_err();
    assert!(!error
        .to_string()
        .contains("no credentials configured for provider ollama"));
}

#[test]
fn execute_anthropic_tool_calls_runs_registered_tools() {
    let resources = LoadedResources {
        tools: vec![loaded_tool("bash", "Run shell", "bash")],
        ..LoadedResources::default()
    };
    let registry = ToolRegistry::from_resources(&resources);
    let response = json!({
        "content": [
            {
                "type": "tool_use",
                "id": "toolu_1",
                "name": "bash",
                "input": {
                    "command": "printf hi"
                }
            }
        ]
    });
    let result = execute_anthropic_tool_calls(
        &resources,
        &response,
        &registry,
        std::env::current_dir().unwrap().as_path(),
    )
    .unwrap();
    assert!(result.is_some());
}

#[test]
fn openai_tool_definitions_use_registry_schema() {
    let resources = LoadedResources {
        tools: vec![loaded_tool("search_text", "Search", "search_text")],
        ..LoadedResources::default()
    };
    let registry = ToolRegistry::from_resources(&resources);
    let tools = openai_tool_definitions(&registry);
    assert_eq!(tools[0].name, "search_text");
    assert_eq!(tools[0].kind, "function");
    assert_eq!(tools[0].parameters["type"], "object");
}

#[test]
fn resolve_openai_execution_config_uses_codex_chatgpt_route_for_builtin_oauth() {
    let mut auth_store = AuthStore::default();
    auth_store.set_oauth(
        "openai",
        OAuthCredential {
            access_token: fake_jwt(json!({
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acct-123"
                }
            })),
            refresh_token: "refresh-123".to_string(),
            expires_at_ms: 42,
            account_id: Some("acct-123".to_string()),
            organization_id: None,
            email: Some("dev@example.com".to_string()),
            plan_type: Some("pro".to_string()),
            rate_limit_tier: None,
            scopes: vec!["openid".to_string()],
        },
    );

    let config = resolve_openai_execution_config(
        &state(),
        &auth_store,
        &openai_provider("https://api.openai.com".to_string()),
    )
    .unwrap();

    assert_eq!(config.request_config.base_url, OPENAI_CHATGPT_BASE_URL);
    assert_eq!(
        config.request_config.account_id.as_deref(),
        Some("acct-123")
    );
    assert!(config.codex_style);
    assert!(config
        .request_config
        .custom_headers
        .iter()
        .any(|(key, _)| key == "version"));
}

#[test]
fn build_codex_openai_request_body_matches_codex_shape() {
    let state = state();
    let body = build_codex_openai_request_body(
        &state,
        "gpt-5",
        Value::String("hello".to_string()),
        &Vec::new(),
        None,
        true,
    );

    assert_eq!(body["model"], json!("gpt-5"));
    assert_eq!(body["stream"], json!(true));
    assert_eq!(
        body["include"][0],
        json!("reasoning.encrypted_content")
    );
    assert_eq!(body["prompt_cache_key"], json!(Uuid::nil().to_string()));
    assert_eq!(body["input"][0]["type"], json!("message"));
    assert_eq!(body["input"][0]["content"][0]["text"], json!("hello"));
    assert_eq!(body["reasoning"]["summary"], json!("auto"));
    assert_eq!(body["reasoning"]["effort"], json!("medium"));
}

#[test]
fn parse_openai_sse_response_reconstructs_output_items() {
    let stream = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_123\"}}\n\n",
        "event: response.output_item.done\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello\"}]}}\n\n",
        "event: response.output_item.done\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"call_id\":\"call_123\",\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\"}}\n\n"
    );

    let parsed = parse_openai_sse_response(stream).unwrap();
    assert_eq!(parsed["id"], json!("resp_123"));
    assert_eq!(parsed["output"].as_array().map(Vec::len), Some(2));
    assert_eq!(parsed["output"][0]["type"], json!("message"));
    assert_eq!(parsed["output"][1]["type"], json!("function_call"));
}

#[test]
fn execute_user_prompt_refreshes_openai_oauth_after_401() {
    let _guard = refresh_env_lock().lock().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_log = Arc::clone(&requests);

    let initial_access_token = fake_jwt(json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "acct-123"
        }
    }));
    let refreshed_access_token = fake_jwt(json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "acct-123"
        }
    }));
    let refreshed_id_token = fake_jwt(json!({
        "email": "dev@example.com",
        "https://api.openai.com/auth": {
            "chatgpt_plan_type": "pro"
        }
    }));
    let refreshed_access_token_for_server = refreshed_access_token.clone();
    let refreshed_id_token_for_server = refreshed_id_token.clone();
    let refresh_url = format!("http://{address}/oauth/token");
    std::env::set_var("CODEX_REFRESH_TOKEN_URL_OVERRIDE", &refresh_url);

    let server = thread::spawn(move || {
        for index in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 8192];
            let bytes = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
            request_log.lock().unwrap().push(request);

            let (status, content_type, body) = match index {
                0 => (
                    401,
                    "application/json",
                    json!({ "error": "unauthorized" }).to_string(),
                ),
                1 => (
                    200,
                    "application/json",
                    json!({
                        "access_token": refreshed_access_token_for_server,
                        "refresh_token": "refresh-2",
                        "expires_in": 3600,
                        "id_token": refreshed_id_token_for_server,
                    })
                    .to_string(),
                ),
                _ => (
                    200,
                    "text/event-stream",
                    concat!(
                        "event: response.created\n",
                        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
                        "event: response.output_item.done\n",
                        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"refreshed ok\"}]}}\n\n",
                        "event: response.completed\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n"
                    )
                    .to_string(),
                ),
            };
            let response = format!(
                "HTTP/1.1 {status} {}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                if status == 200 { "OK" } else { "Unauthorized" },
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });

    let mut registry = ProviderRegistry::new();
    registry.register(openai_provider(format!("http://{address}/api/codex")));
    let mut auth_store = AuthStore::default();
    auth_store.set_oauth(
        "openai",
        OAuthCredential {
            access_token: initial_access_token.clone(),
            refresh_token: "refresh-1".to_string(),
            expires_at_ms: 0,
            account_id: Some("acct-123".to_string()),
            organization_id: None,
            email: None,
            plan_type: None,
            rate_limit_tier: None,
            scopes: vec!["openid".to_string()],
        },
    );
    let mut state = state();
    state.current_provider = Some("openai".to_string());
    state.current_model = Some("openai/gpt-5".to_string());

    let turn = execute_user_prompt(
        &state,
        &LoadedResources::default(),
        &registry,
        &mut auth_store,
        "hello",
    )
    .unwrap();
    std::env::remove_var("CODEX_REFRESH_TOKEN_URL_OVERRIDE");
    server.join().unwrap();

    assert_eq!(turn.assistant_text, "refreshed ok");
    let stored = match auth_store.get("openai") {
        Some(StoredCredential::OAuth(credential)) => credential,
        other => panic!("expected oauth credential, got {other:?}"),
    };
    assert_eq!(stored.access_token, refreshed_access_token);
    assert_eq!(stored.refresh_token, "refresh-2");
    assert_eq!(stored.email.as_deref(), Some("dev@example.com"));
    assert_eq!(stored.plan_type.as_deref(), Some("pro"));

    let requests = requests.lock().unwrap();
    let first = requests[0].to_ascii_lowercase();
    let second = requests[1].to_ascii_lowercase();
    let third = requests[2].to_ascii_lowercase();
    assert!(first.contains("post /api/codex/responses http/1.1"));
    assert!(first.contains(&format!(
        "authorization: bearer {}",
        initial_access_token.to_ascii_lowercase()
    )));
    assert!(first.contains("originator: codex_cli_rs"));
    assert!(second.contains("post /oauth/token http/1.1"));
    assert!(third.contains(&format!(
        "authorization: bearer {}",
        refreshed_access_token.to_ascii_lowercase()
    )));
}

#[test]
fn execute_openai_tool_calls_serializes_outputs() {
    let resources = LoadedResources {
        tools: vec![loaded_tool("bash", "Run shell", "bash")],
        ..LoadedResources::default()
    };
    let registry = ToolRegistry::from_resources(&resources);
    let tool_calls = vec![OpenAIResponseToolCall {
        item_id: Some("fc_1".to_string()),
        status: Some("completed".to_string()),
        call_id: "call_1".to_string(),
        name: "bash".to_string(),
        arguments: json!({ "command": "printf hi" }),
    }];
    let result = execute_openai_tool_calls(
        &resources,
        &tool_calls,
        &registry,
        std::env::current_dir().unwrap().as_path(),
    )
    .unwrap();
    assert_eq!(result.outputs[0].kind, "function_call_output");
    assert_eq!(result.outputs[0].call_id, "call_1");
    assert!(result.outputs[0].output.contains("hi"));
    assert_eq!(result.invocations[0].tool_id, "bash");
}

#[test]
fn tool_hooks_run_for_completed_tool_calls() {
    let temp = tempfile::tempdir().unwrap();
    let hook_output = temp.path().join("hook.txt");
    let resources = LoadedResources {
        hooks: vec![LoadedItem {
            value: puffer_resources::HookSpec {
                id: "tool-end".to_string(),
                event: "tool_end".to_string(),
                command: format!("printf \"$PUFFER_TOOL_ID\" > {}", hook_output.display()),
            },
            source_info: SourceInfo {
                path: "hook.yaml".into(),
                kind: SourceKind::Builtin,
            },
        }],
        tools: vec![loaded_tool("bash", "Run shell", "bash")],
        ..LoadedResources::default()
    };
    let registry = ToolRegistry::from_resources(&resources);
    let response = json!({
        "content": [
            {
                "type": "tool_use",
                "id": "toolu_1",
                "name": "bash",
                "input": {
                    "command": "printf hi"
                }
            }
        ]
    });
    let _ = execute_anthropic_tool_calls(&resources, &response, &registry, temp.path()).unwrap();
    assert_eq!(std::fs::read_to_string(hook_output).unwrap(), "bash");
}

#[test]
fn transcript_to_anthropic_messages_replays_all_roles() {
    let mut state = state();
    state.push_message(crate::MessageRole::User, "hello");
    state.push_message(crate::MessageRole::Assistant, "hi");
    state.push_message(crate::MessageRole::System, "note");

    let messages = transcript_to_anthropic_messages(&state, "fallback");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[2]["content"], "[system]\nnote");
}

#[test]
fn transcript_to_openai_input_replays_transcript_items() {
    let mut state = state();
    state.push_message(crate::MessageRole::User, "hello");
    state.push_message(crate::MessageRole::Assistant, "hi");

    let input = transcript_to_openai_input(&state, "fallback");
    let items = input.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["role"], "user");
    assert_eq!(items[1]["type"], "message");
    assert_eq!(items[1]["role"], "assistant");
}

#[test]
fn transcript_to_openai_chat_messages_replays_transcript_items() {
    let mut state = state();
    state.push_message(crate::MessageRole::User, "hello");
    state.push_message(crate::MessageRole::Assistant, "hi");

    let messages = transcript_to_openai_chat_messages(&state, "fallback");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[0].content, Some(json!("hello")));
}

#[test]
fn transcript_to_openai_chat_messages_preserves_system_role() {
    let mut state = state();
    state.push_message(crate::MessageRole::System, "rules");
    state.push_message(crate::MessageRole::User, "hello");
    state.push_message(crate::MessageRole::Assistant, "hi");

    let messages = transcript_to_openai_chat_messages(&state, "fallback");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[2].role, "assistant");
}
