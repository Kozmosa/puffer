use super::*;
use puffer_provider_registry::AuthMode;

fn provider(
    id: &str,
    api: &str,
    auth_modes: Vec<AuthMode>,
) -> puffer_provider_registry::ProviderDescriptor {
    puffer_provider_registry::ProviderDescriptor {
        id: id.to_string(),
        display_name: id.to_string(),
        base_url: "https://example.invalid".to_string(),
        default_api: api.to_string(),
        auth_modes,
        headers: Default::default(),
        query_params: Default::default(),
        discovery: None,
        models: vec![puffer_provider_registry::ModelDescriptor {
            id: "model".to_string(),
            display_name: "model".to_string(),
            provider: id.to_string(),
            api: api.to_string(),
            context_window: 1000,
            max_output_tokens: 100,
            supports_reasoning: false,
        }],
    }
}

#[test]
fn login_command_reports_provider_auth_modes_and_family_hint() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store
        .create_session(tempdir.path().to_path_buf())
        .unwrap();
    let mut state = AppState::new(
        PufferConfig::default(),
        tempdir.path().to_path_buf(),
        session,
    );
    let mut providers = ProviderRegistry::new();
    providers.register(provider(
        "custom-openai",
        "openai-responses",
        vec![AuthMode::ApiKey, AuthMode::OAuth],
    ));

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut providers,
        &mut AuthStore::default(),
        &session_store,
        "/login custom-openai",
    )
    .unwrap();

    assert!(matches!(
        state.transcript.last(),
        Some(RenderedMessage {
            role: MessageRole::System,
            text,
        }) if text.contains("Supported auth modes: api_key, oauth")
            && text.contains("API key: `puffer auth set-api-key custom-openai --stdin`")
            && text.contains("OAuth start bundle:")
            && text.contains("url=")
            && text.contains("verifier=")
    ));
}

#[test]
fn login_command_reports_session_ingress_support() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store
        .create_session(tempdir.path().to_path_buf())
        .unwrap();
    let mut state = AppState::new(
        PufferConfig::default(),
        tempdir.path().to_path_buf(),
        session,
    );
    let mut providers = ProviderRegistry::new();
    providers.register(provider(
        "custom-anthropic",
        "anthropic-messages",
        vec![AuthMode::ApiKey, AuthMode::SessionIngress],
    ));

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut providers,
        &mut AuthStore::default(),
        &session_store,
        "/login custom-anthropic",
    )
    .unwrap();

    assert!(matches!(
        state.transcript.last(),
        Some(RenderedMessage {
            role: MessageRole::System,
            text,
        }) if text.contains("session_ingress")
            && text.contains("Session ingress: exported session-ingress credentials are supported.")
    ));
}

#[test]
fn login_command_reports_when_provider_has_no_auth_modes() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store
        .create_session(tempdir.path().to_path_buf())
        .unwrap();
    let mut state = AppState::new(
        PufferConfig::default(),
        tempdir.path().to_path_buf(),
        session,
    );
    let mut providers = ProviderRegistry::new();
    providers.register(provider("ollama", "openai-completions", Vec::new()));

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut providers,
        &mut AuthStore::default(),
        &session_store,
        "/login ollama",
    )
    .unwrap();

    assert!(matches!(
        state.transcript.last(),
        Some(RenderedMessage {
            role: MessageRole::System,
            text,
        }) if text == "ollama does not require stored credentials."
    ));
}
