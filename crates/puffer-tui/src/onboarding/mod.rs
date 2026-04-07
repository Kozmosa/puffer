use crate::state::{
    load_agent_picker_entries, AuthPickerAction, AuthPickerEntry, ModelPickerEntry, OverlayState,
};
use crate::usage::UsageOverlay;
use anyhow::Result;
use puffer_config::ConfigPaths;
use puffer_core::AppState;
use puffer_provider_registry::{
    detect_import_candidates, AuthMode, AuthStore, ExternalImportCandidate, ExternalImportFamily,
    ExternalImportSource, ProviderDescriptor, ProviderRegistry,
};
use puffer_session_store::SessionStore;

/// Returns whether the current session still needs a selected provider/model pair.
pub(crate) fn needs_initial_provider_setup(state: &AppState, providers: &ProviderRegistry) -> bool {
    let Some(provider_id) = state.current_provider.as_deref() else {
        return true;
    };
    let Some(provider) = providers.provider(provider_id) else {
        return true;
    };
    if provider.models.is_empty() {
        return true;
    }
    let Some(model_selector) = state.current_model.as_deref() else {
        return false;
    };
    let Some((selected_provider, selected_model)) = model_selector.split_once('/') else {
        return false;
    };
    selected_provider != provider_id
        || !provider
            .models
            .iter()
            .any(|model| model.id == selected_model)
}

/// Builds the initial onboarding overlay when provider/model setup is incomplete.
pub(crate) fn initial_overlay(
    state: &AppState,
    providers: &mut ProviderRegistry,
    auth_store: &AuthStore,
) -> Result<Option<OverlayState>> {
    if !needs_initial_provider_setup(state, providers)
        && !missing_required_auth(state, providers, auth_store)
    {
        return Ok(None);
    }
    let paths = ConfigPaths::discover(&state.cwd);
    if !paths.has_user_config() {
        return Ok(Some(theme_picker()));
    }
    if let Some(provider_id) = state.current_provider.as_deref() {
        return provider_setup_overlay(providers, auth_store, provider_id);
    }
    Ok(provider_picker(providers, true))
}

/// Builds a command-driven picker overlay, including provider-scoped model selection.
pub(crate) fn overlay_from_command(
    state: &AppState,
    providers: &mut ProviderRegistry,
    auth_store: &AuthStore,
    session_store: &SessionStore,
    submitted: &str,
) -> Result<Option<OverlayState>> {
    let without_slash = submitted.trim_start_matches('/');
    let (name, args) = without_slash
        .split_once(' ')
        .map(|(name, args)| (name, args.trim()))
        .unwrap_or((without_slash, ""));

    let overlay = match name {
        "resume" | "continue" if args.is_empty() => {
            let sessions = session_store.list_sessions()?;
            if sessions.is_empty() {
                None
            } else {
                Some(OverlayState::SessionPicker {
                    sessions,
                    selection: 0,
                })
            }
        }
        "model" if args.is_empty() => state.current_provider.as_deref().and_then(|provider_id| {
            refresh_provider_models_best_effort(providers, auth_store, provider_id);
            model_picker(providers, provider_id, false)
        }),
        "agents" if args.is_empty() => {
            let entries = load_agent_picker_entries(&state.cwd, state.current_model.as_deref())?;
            if entries.is_empty() {
                None
            } else {
                Some(OverlayState::AgentPicker {
                    entries,
                    selection: 0,
                })
            }
        }
        "login" if args.is_empty() => login_provider_picker(providers),
        "login" if !args.is_empty() => auth_picker(providers, auth_store, args, false)?,
        "logout" if args.is_empty() => logout_picker(providers, auth_store),
        "theme" if args.is_empty() => Some(theme_picker()),
        "usage" if args.is_empty() => Some(UsageOverlay::open(state, providers, auth_store)),
        _ => None,
    };
    Ok(overlay)
}

/// Builds the next auth-or-model overlay for one provider.
pub(crate) fn provider_setup_overlay(
    providers: &mut ProviderRegistry,
    auth_store: &AuthStore,
    provider_id: &str,
) -> Result<Option<OverlayState>> {
    let Some(provider) = providers.provider(provider_id) else {
        return Ok(None);
    };
    if needs_auth_choice(provider, auth_store) {
        return auth_picker(providers, auth_store, provider_id, true);
    }
    refresh_provider_models_best_effort(providers, auth_store, provider_id);
    Ok(model_picker(providers, provider_id, true))
}

/// Returns the first provider step used after the initial theme selection.
pub(crate) fn initial_provider_overlay(providers: &ProviderRegistry) -> Option<OverlayState> {
    provider_picker(providers, true)
}

/// Returns the previous overlay to show when the user presses Escape.
pub(crate) fn back_overlay(
    overlay: &OverlayState,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
) -> Result<Option<OverlayState>> {
    let next = match overlay {
        OverlayState::ApiKeyPrompt {
            provider_id,
            onboarding,
            ..
        } => auth_picker(providers, auth_store, provider_id, *onboarding)?,
        OverlayState::AuthPicker { onboarding, .. } => provider_picker(providers, *onboarding),
        OverlayState::ProviderPicker { onboarding, .. } if *onboarding => Some(theme_picker()),
        OverlayState::ModelPicker { onboarding, .. } if *onboarding => {
            provider_picker(providers, true)
        }
        OverlayState::SessionPicker { .. }
        | OverlayState::AgentPicker { .. }
        | OverlayState::ModelPicker { .. }
        | OverlayState::ProviderPicker { .. }
        | OverlayState::LoginPicker { .. }
        | OverlayState::LogoutPicker { .. }
        | OverlayState::ThemePicker { .. }
        | OverlayState::CommandPicker { .. }
        | OverlayState::OnboardingTheme { .. }
        | OverlayState::OnboardingProvider { .. }
        | OverlayState::OnboardingAuth { .. }
        | OverlayState::OnboardingModel { .. }
        | OverlayState::OnboardingApiKey { .. }
        | OverlayState::Usage(..) => None,
    };
    Ok(next)
}

fn missing_required_auth(
    state: &AppState,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
) -> bool {
    let Some(provider_id) = state.current_provider.as_deref() else {
        return false;
    };
    let Some(provider) = providers.provider(provider_id) else {
        return false;
    };
    needs_auth_choice(provider, auth_store)
}

fn needs_auth_choice(provider: &ProviderDescriptor, auth_store: &AuthStore) -> bool {
    let supports_local_auth = provider.auth_modes.contains(&AuthMode::OAuth)
        || provider.auth_modes.contains(&AuthMode::ApiKey);
    supports_local_auth && !auth_store.has_auth(&provider.id)
}

fn provider_picker(providers: &ProviderRegistry, onboarding: bool) -> Option<OverlayState> {
    let mut providers = providers
        .providers()
        .filter(|provider| {
            !provider.models.is_empty() && (onboarding || !provider.auth_modes.is_empty())
        })
        .collect::<Vec<_>>();
    providers.sort_by_key(|provider| provider_rank(provider.id.as_str()));
    let entries = providers
        .into_iter()
        .map(provider_entry)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    if onboarding {
        Some(OverlayState::ProviderPicker {
            entries,
            selection: 0,
            onboarding,
        })
    } else {
        Some(OverlayState::LoginPicker {
            entries,
            selection: 0,
        })
    }
}

fn login_provider_picker(providers: &ProviderRegistry) -> Option<OverlayState> {
    let mut providers = providers
        .providers()
        .filter(|provider| !provider.models.is_empty())
        .filter(|provider| !provider.auth_modes.is_empty())
        .collect::<Vec<_>>();
    providers.sort_by_key(|provider| provider_rank(provider.id.as_str()));
    let entries = providers
        .into_iter()
        .map(|provider| ModelPickerEntry {
            selector: provider.id.clone(),
            description: provider.display_name.clone(),
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        None
    } else {
        Some(OverlayState::LoginPicker {
            entries,
            selection: 0,
        })
    }
}

fn provider_entry(provider: &ProviderDescriptor) -> ModelPickerEntry {
    ModelPickerEntry {
        selector: provider.id.clone(),
        description: provider.display_name.clone(),
    }
}

fn auth_picker(
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    provider_id: &str,
    onboarding: bool,
) -> Result<Option<OverlayState>> {
    let Some(provider) = providers.provider(provider_id) else {
        return Ok(None);
    };

    let mut entries = Vec::new();
    if auth_store.has_auth(provider_id) {
        entries.push(AuthPickerEntry {
            label: "stored".to_string(),
            description: "Use the stored credentials in ~/.puffer/auth.json and secure storage"
                .to_string(),
            action: AuthPickerAction::UseStored,
        });
    }
    if provider.auth_modes.contains(&AuthMode::OAuth) {
        entries.push(AuthPickerEntry {
            label: "oauth".to_string(),
            description: "Sign in with your browser".to_string(),
            action: AuthPickerAction::OAuth,
        });
    }
    if provider.auth_modes.contains(&AuthMode::ApiKey) {
        entries.push(AuthPickerEntry {
            label: "api-key".to_string(),
            description: "Paste and store an API key".to_string(),
            action: AuthPickerAction::ApiKey,
        });
    }
    for candidate in import_candidates(provider)? {
        entries.push(AuthPickerEntry {
            label: import_label(&candidate),
            description: candidate.description.clone(),
            action: AuthPickerAction::Import(candidate),
        });
    }
    if entries.is_empty() {
        entries.push(AuthPickerEntry {
            label: "continue".to_string(),
            description: "This provider does not require stored credentials".to_string(),
            action: AuthPickerAction::NoneRequired,
        });
    }
    Ok(Some(OverlayState::AuthPicker {
        provider_id: provider.id.clone(),
        entries,
        selection: 0,
        onboarding,
    }))
}

fn import_candidates(provider: &ProviderDescriptor) -> Result<Vec<ExternalImportCandidate>> {
    let Some(family) = import_family(provider) else {
        return Ok(Vec::new());
    };
    detect_import_candidates(family)
}

fn import_family(provider: &ProviderDescriptor) -> Option<ExternalImportFamily> {
    match provider.default_api.as_str() {
        "anthropic-messages" => Some(ExternalImportFamily::Anthropic),
        "openai-responses"
        | "openai-completions"
        | "openai-codex-responses"
        | "azure-openai-responses" => Some(ExternalImportFamily::OpenAi),
        _ => None,
    }
}

fn import_label(candidate: &ExternalImportCandidate) -> String {
    match candidate.source {
        ExternalImportSource::Claude => "import-claude".to_string(),
        ExternalImportSource::Codex => "import-codex".to_string(),
    }
}

fn refresh_provider_models_best_effort(
    providers: &mut ProviderRegistry,
    auth_store: &AuthStore,
    provider_id: &str,
) {
    let _ = providers.discover_and_merge_provider(provider_id, auth_store);
}

fn model_picker(
    providers: &ProviderRegistry,
    provider_id: &str,
    onboarding: bool,
) -> Option<OverlayState> {
    let provider = providers.provider(provider_id)?;
    let entries = provider
        .models
        .iter()
        .map(|model| ModelPickerEntry {
            selector: model.id.clone(),
            description: model.display_name.clone(),
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        None
    } else {
        Some(OverlayState::ModelPicker {
            provider_id: provider.id.clone(),
            entries,
            selection: 0,
            onboarding,
        })
    }
}

fn logout_picker(providers: &ProviderRegistry, auth_store: &AuthStore) -> Option<OverlayState> {
    let entries = auth_store
        .provider_ids()
        .map(|provider_id| ModelPickerEntry {
            selector: provider_id.to_string(),
            description: providers
                .provider(provider_id)
                .map(|provider| provider.display_name.clone())
                .unwrap_or_else(|| provider_id.to_string()),
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        None
    } else {
        Some(OverlayState::LogoutPicker {
            entries,
            selection: 0,
        })
    }
}

fn theme_picker() -> OverlayState {
    OverlayState::ThemePicker {
        entries: vec![
            ModelPickerEntry {
                selector: "puffer".to_string(),
                description: "Default Puffer text style".to_string(),
            },
            ModelPickerEntry {
                selector: "harbor".to_string(),
                description: "Calmer contrast for long sessions".to_string(),
            },
            ModelPickerEntry {
                selector: "sunrise".to_string(),
                description: "Warmer light-on-dark palette".to_string(),
            },
        ],
        selection: 0,
    }
}

fn provider_rank(provider_id: &str) -> (u8, &str) {
    match provider_id {
        "anthropic" => (0, provider_id),
        "openai" | "openai-codex" | "azure-openai-responses" => (1, provider_id),
        _ => (2, provider_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_provider_registry::{
        AuthMode, ModelDescriptor, ModelDiscoveryConfig, ModelDiscoveryFormat,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn openai_provider(address: std::net::SocketAddr) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "openai".to_string(),
            display_name: "OpenAI".to_string(),
            base_url: format!("http://{address}/api/codex"),
            default_api: "openai-responses".to_string(),
            auth_modes: vec![AuthMode::OAuth],
            headers: Default::default(),
            query_params: Default::default(),
            discovery: Some(ModelDiscoveryConfig {
                path: "/v1/models".to_string(),
                response: ModelDiscoveryFormat::OpenAiModels,
                api: "openai-responses".to_string(),
                context_window: 272_000,
                max_output_tokens: 16_384,
                supports_reasoning: true,
                items_field: "data".to_string(),
                id_field: "id".to_string(),
                display_name_field: None,
                headers: Default::default(),
            }),
            models: vec![ModelDescriptor {
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

    fn spawn_model_server() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("connection");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer).expect("request");
            let body = serde_json::json!({
                "models": [
                    {
                        "slug": "gpt-5",
                        "display_name": "gpt-5",
                        "supported_in_api": true,
                        "visibility": "list",
                        "supported_reasoning_levels": [
                            { "effort": "medium", "description": "medium" }
                        ],
                        "context_window": 272000
                    },
                    {
                        "slug": "gpt-4.1",
                        "display_name": "gpt-4.1",
                        "supported_in_api": true,
                        "visibility": "list",
                        "supported_reasoning_levels": [
                            { "effort": "medium", "description": "medium" }
                        ],
                        "context_window": 128000
                    },
                    {
                        "slug": "gpt-4.1-mini",
                        "display_name": "gpt-4.1-mini",
                        "supported_in_api": true,
                        "visibility": "list",
                        "supported_reasoning_levels": [
                            { "effort": "medium", "description": "medium" }
                        ],
                        "context_window": 128000
                    }
                ]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response write");
        });
        address
    }

    #[test]
    fn provider_setup_overlay_refreshes_models_after_auth() {
        let address = spawn_model_server();
        let mut providers = ProviderRegistry::new();
        providers.register(openai_provider(address));
        let mut auth_store = AuthStore::default();
        auth_store.set_oauth(
            "openai",
            puffer_provider_registry::OAuthCredential {
                access_token: "token".to_string(),
                ..Default::default()
            },
        );

        let overlay =
            provider_setup_overlay(&mut providers, &auth_store, "openai").expect("overlay result");

        match overlay {
            Some(OverlayState::ModelPicker { entries, .. }) => {
                assert!(entries.iter().any(|entry| entry.selector == "gpt-4.1"));
                assert!(entries.iter().any(|entry| entry.selector == "gpt-4.1-mini"));
            }
            other => panic!("expected model picker, got {other:?}"),
        }
    }

    #[test]
    fn overlay_from_model_command_refreshes_active_provider_models() {
        let address = spawn_model_server();
        let mut providers = ProviderRegistry::new();
        providers.register(openai_provider(address));
        let mut auth_store = AuthStore::default();
        auth_store.set_oauth(
            "openai",
            puffer_provider_registry::OAuthCredential {
                access_token: "token".to_string(),
                ..Default::default()
            },
        );
        let mut state = AppState::new(
            puffer_config::PufferConfig::default(),
            std::path::PathBuf::from("/workspace/puffer"),
            puffer_session_store::SessionMetadata {
                id: uuid::Uuid::nil(),
                display_name: None,
                cwd: std::path::PathBuf::from("/workspace/puffer"),
                created_at_ms: 0,
                updated_at_ms: 0,
                parent_session_id: None,
                slug: None,
                tags: Vec::new(),
                note: None,
            },
        );
        state.current_provider = Some("openai".to_string());
        let tempdir = tempfile::tempdir().expect("tempdir");
        let session_store = SessionStore::from_paths(&ConfigPaths::discover(tempdir.path()))
            .expect("session store");

        let overlay = overlay_from_command(
            &state,
            &mut providers,
            &auth_store,
            &session_store,
            "/model",
        )
        .expect("overlay result");

        match overlay {
            Some(OverlayState::ModelPicker { entries, .. }) => {
                assert!(entries.iter().any(|entry| entry.selector == "gpt-4.1"));
                assert!(entries.iter().any(|entry| entry.selector == "gpt-4.1-mini"));
            }
            other => panic!("expected model picker, got {other:?}"),
        }
    }
}
