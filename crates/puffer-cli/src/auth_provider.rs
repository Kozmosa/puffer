use anyhow::{anyhow, Result};
use puffer_provider_openai::{
    build_authorization_url as build_openai_authorization_url,
    generate_pkce as generate_openai_pkce, OpenAIOAuthConfig,
};
use puffer_provider_registry::{AuthMode, ProviderRegistry};
use puffer_transport_anthropic::{
    build_authorization_url as build_anthropic_authorization_url,
    generate_pkce as generate_anthropic_pkce, AnthropicOAuthConfig, CONSOLE_AUTHORIZE_URL,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OauthFamily {
    Anthropic,
    OpenAi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OauthStartBundle {
    pub(crate) authorization_url: String,
    pub(crate) verifier: String,
    pub(crate) state: String,
    pub(crate) redirect_uri: String,
}

/// Returns the OAuth family to use for a provider when one is supported.
pub(crate) fn oauth_family_for_provider(
    providers: &ProviderRegistry,
    provider_id: &str,
) -> Option<OauthFamily> {
    let provider = providers.provider(provider_id)?;
    if !provider.auth_modes.contains(&AuthMode::OAuth) {
        return None;
    }
    match provider.default_api.as_str() {
        "openai-responses"
        | "openai-completions"
        | "azure-openai-responses"
        | "openai-codex-responses" => Some(OauthFamily::OpenAi),
        "anthropic-messages" => Some(OauthFamily::Anthropic),
        _ => None,
    }
}

/// Builds a provider-specific OAuth start bundle using the provider API family.
pub(crate) fn oauth_start_bundle_for_provider(
    providers: &ProviderRegistry,
    provider_id: &str,
) -> Result<OauthStartBundle> {
    match oauth_family_for_provider(providers, provider_id) {
        Some(OauthFamily::OpenAi) => {
            let pkce = generate_openai_pkce();
            let config = OpenAIOAuthConfig {
                state: pkce.state.clone(),
                code_challenge: pkce.challenge.clone(),
                ..OpenAIOAuthConfig::default()
            };
            Ok(OauthStartBundle {
                authorization_url: build_openai_authorization_url(&config),
                verifier: pkce.verifier,
                state: pkce.state,
                redirect_uri: config.redirect_uri,
            })
        }
        Some(OauthFamily::Anthropic) => {
            let pkce = generate_anthropic_pkce();
            let mut config = AnthropicOAuthConfig {
                state: pkce.state.clone(),
                code_challenge: pkce.challenge.clone(),
                ..AnthropicOAuthConfig::default()
            };
            if provider_id != "anthropic" {
                config.authorize_url = CONSOLE_AUTHORIZE_URL.to_string();
            }
            Ok(OauthStartBundle {
                authorization_url: build_anthropic_authorization_url(&config),
                verifier: pkce.verifier,
                state: pkce.state,
                redirect_uri: config.redirect_uri,
            })
        }
        None => Err(anyhow!("oauth is not implemented for {provider_id}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_provider_registry::{ModelDescriptor, ProviderDescriptor};

    fn provider(id: &str, api: &str, auth_modes: Vec<AuthMode>) -> ProviderDescriptor {
        ProviderDescriptor {
            id: id.to_string(),
            display_name: id.to_string(),
            base_url: "https://example.invalid".to_string(),
            default_api: api.to_string(),
            auth_modes,
            headers: Default::default(),
            query_params: Default::default(),
            discovery: None,
            models: vec![ModelDescriptor {
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
    fn oauth_family_uses_provider_api_family() {
        let mut providers = ProviderRegistry::new();
        providers.register(provider(
            "custom-openai",
            "openai-responses",
            vec![AuthMode::OAuth],
        ));
        providers.register(provider(
            "custom-openai-codex",
            "openai-codex-responses",
            vec![AuthMode::OAuth],
        ));
        providers.register(provider(
            "custom-azure-openai",
            "azure-openai-responses",
            vec![AuthMode::OAuth],
        ));
        providers.register(provider(
            "custom-anthropic",
            "anthropic-messages",
            vec![AuthMode::OAuth],
        ));
        assert_eq!(
            oauth_family_for_provider(&providers, "custom-openai"),
            Some(OauthFamily::OpenAi)
        );
        assert_eq!(
            oauth_family_for_provider(&providers, "custom-openai-codex"),
            Some(OauthFamily::OpenAi)
        );
        assert_eq!(
            oauth_family_for_provider(&providers, "custom-azure-openai"),
            Some(OauthFamily::OpenAi)
        );
        assert_eq!(
            oauth_family_for_provider(&providers, "custom-anthropic"),
            Some(OauthFamily::Anthropic)
        );
    }

    #[test]
    fn oauth_family_requires_oauth_auth_mode() {
        let mut providers = ProviderRegistry::new();
        providers.register(provider(
            "custom-openai",
            "openai-responses",
            vec![AuthMode::ApiKey],
        ));
        assert_eq!(oauth_family_for_provider(&providers, "custom-openai"), None);
    }
}
