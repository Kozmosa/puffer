use super::*;
use puffer_provider_registry::AuthMode;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;

fn puffer_home_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_puffer_home() -> MutexGuard<'static, ()> {
    puffer_home_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct ScopedPufferHome {
    old_home: Option<OsString>,
}

impl ScopedPufferHome {
    fn set(path: &std::path::Path) -> Self {
        let old_home = std::env::var_os("PUFFER_HOME");
        std::env::set_var("PUFFER_HOME", path);
        Self { old_home }
    }
}

impl Drop for ScopedPufferHome {
    fn drop(&mut self) {
        if let Some(value) = self.old_home.take() {
            std::env::set_var("PUFFER_HOME", value);
        } else {
            std::env::remove_var("PUFFER_HOME");
        }
    }
}

fn provider(id: &str, models: &[&str]) -> puffer_provider_registry::ProviderDescriptor {
    puffer_provider_registry::ProviderDescriptor {
        id: id.to_string(),
        display_name: id.to_string(),
        base_url: "https://example.invalid".to_string(),
        default_api: "openai-responses".to_string(),
        auth_modes: vec![AuthMode::ApiKey],
        headers: Default::default(),
        query_params: Default::default(),
        discovery: None,
        models: models
            .iter()
            .map(|model_id| puffer_provider_registry::ModelDescriptor {
                id: (*model_id).to_string(),
                display_name: (*model_id).to_string(),
                provider: id.to_string(),
                api: "openai-responses".to_string(),
                context_window: 1000,
                max_output_tokens: 100,
                supports_reasoning: false,
            })
            .collect(),
    }
}

fn provider_with_discovery(
    id: &str,
    base_url: String,
    models: &[&str],
) -> puffer_provider_registry::ProviderDescriptor {
    let mut provider = provider(id, models);
    provider.base_url = base_url;
    provider.discovery = Some(puffer_provider_registry::ModelDiscoveryConfig {
        path: "/v1/models".to_string(),
        response: puffer_provider_registry::ModelDiscoveryFormat::OpenAiModels,
        api: "openai-responses".to_string(),
        context_window: 272_000,
        max_output_tokens: 16_384,
        supports_reasoning: true,
        items_field: "data".to_string(),
        id_field: "id".to_string(),
        display_name_field: None,
        headers: Default::default(),
    });
    provider
}

fn spawn_model_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 4096];
        let _ = stream.read(&mut buffer).unwrap();
        let body = serde_json::json!({
            "data": [
                { "id": "gpt-5" },
                { "id": "gpt-4.1" },
                { "id": "gpt-4.1-mini" }
            ]
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    address
}

#[test]
fn model_command_lists_only_models_for_selected_provider() {
    let tempdir = tempdir().unwrap();
    let _lock = lock_puffer_home();
    let home = tempdir.path().join("home");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let _home = ScopedPufferHome::set(&home);
    let paths = ConfigPaths::discover(&workspace);
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store.create_session(workspace.clone()).unwrap();
    let mut config = PufferConfig::default();
    config.default_provider = Some("openai".to_string());
    config.default_model = Some("openai/gpt-5".to_string());
    let mut state = AppState::new(config, workspace, session);
    state.current_provider = Some("openai".to_string());
    state.current_model = Some("openai/gpt-5".to_string());
    let mut providers = ProviderRegistry::new();
    providers.register(provider("openai", &["gpt-5", "gpt-5-mini"]));
    providers.register(provider("groq", &["compound"]));

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut providers,
        &mut AuthStore::default(),
        &session_store,
        "/model",
    )
    .unwrap();

    assert!(matches!(
        state.transcript.last(),
        Some(RenderedMessage { text, .. })
            if text.contains("Available models for openai: gpt-5, gpt-5-mini")
                && !text.contains("compound")
    ));
}

#[test]
fn model_command_refreshes_active_provider_before_listing() {
    let tempdir = tempdir().unwrap();
    let _lock = lock_puffer_home();
    let home = tempdir.path().join("home");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let _home = ScopedPufferHome::set(&home);
    let paths = ConfigPaths::discover(&workspace);
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store.create_session(workspace.clone()).unwrap();
    let mut config = PufferConfig::default();
    config.default_provider = Some("openai".to_string());
    config.default_model = Some("openai/gpt-5".to_string());
    let mut state = AppState::new(config, workspace, session);
    state.current_provider = Some("openai".to_string());
    state.current_model = Some("openai/gpt-5".to_string());
    let address = spawn_model_server();
    let mut providers = ProviderRegistry::new();
    providers.register(provider_with_discovery(
        "openai",
        format!("http://{address}"),
        &["gpt-5", "gpt-5-mini"],
    ));
    let mut auth_store = AuthStore::default();
    auth_store.set_api_key("openai", "sk-openai");

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut providers,
        &mut auth_store,
        &session_store,
        "/model",
    )
    .unwrap();

    assert!(matches!(
        state.transcript.last(),
        Some(RenderedMessage { text, .. })
            if text.contains("Available models for openai: ")
                && text.contains("gpt-4.1")
                && text.contains("gpt-4.1-mini")
    ));
}

#[test]
fn model_command_rejects_cross_provider_selection() {
    let tempdir = tempdir().unwrap();
    let _lock = lock_puffer_home();
    let home = tempdir.path().join("home");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let _home = ScopedPufferHome::set(&home);
    let paths = ConfigPaths::discover(&workspace);
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store.create_session(workspace.clone()).unwrap();
    let mut config = PufferConfig::default();
    config.default_provider = Some("openai".to_string());
    config.default_model = Some("openai/gpt-5".to_string());
    let mut state = AppState::new(config, workspace, session);
    state.current_provider = Some("openai".to_string());
    state.current_model = Some("openai/gpt-5".to_string());
    let mut providers = ProviderRegistry::new();
    providers.register(provider("openai", &["gpt-5"]));
    providers.register(provider("groq", &["compound"]));

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut providers,
        &mut AuthStore::default(),
        &session_store,
        "/model groq/compound",
    )
    .unwrap();

    assert!(matches!(
        state.transcript.last(),
        Some(RenderedMessage { text, .. })
            if text.contains("/model only selects models for the active provider (openai).")
    ));
}

#[test]
fn model_command_persists_selected_model_to_user_config() {
    let tempdir = tempdir().unwrap();
    let _lock = lock_puffer_home();
    let home = tempdir.path().join("home");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let _home = ScopedPufferHome::set(&home);
    let paths = ConfigPaths::discover(&workspace);
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store.create_session(workspace.clone()).unwrap();
    let mut config = PufferConfig::default();
    config.default_provider = Some("openai".to_string());
    config.default_model = Some("openai/gpt-5".to_string());
    let mut state = AppState::new(config, workspace, session);
    state.current_provider = Some("openai".to_string());
    state.current_model = Some("openai/gpt-5".to_string());
    let mut providers = ProviderRegistry::new();
    providers.register(provider("openai", &["gpt-5", "gpt-5-mini"]));

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut providers,
        &mut AuthStore::default(),
        &session_store,
        "/model gpt-5-mini",
    )
    .unwrap();

    let saved = std::fs::read_to_string(paths.user_config_file()).unwrap();
    assert!(saved.contains("default_provider = \"openai\""));
    assert!(saved.contains("default_model = \"openai/gpt-5-mini\""));
}
