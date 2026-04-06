use crate::auth::{AuthStore, StoredCredential};
use crate::model::{
    ModelDescriptor, ModelDiscoveryConfig, ModelDiscoveryFormat, ProviderDescriptor,
};
use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde_json::Value;

/// Generic runtime model discovery client for provider APIs.
#[derive(Debug, Clone, Default)]
pub struct ModelDiscoveryClient;

impl ModelDiscoveryClient {
    /// Creates a discovery client using the default blocking HTTP transport.
    pub fn new() -> Self {
        Self
    }

    /// Discovers models for a provider using its configured discovery endpoint.
    pub fn discover_models(
        &self,
        provider: &ProviderDescriptor,
        auth_store: &AuthStore,
    ) -> Result<Vec<ModelDescriptor>> {
        let Some(discovery) = provider.discovery.as_ref() else {
            return Ok(Vec::new());
        };

        let url = format!(
            "{}{}",
            provider.base_url.trim_end_matches('/'),
            discovery.path
        );
        let client = Client::new();
        let mut request = client.get(&url);
        for (key, value) in &provider.headers {
            request = request.header(key, value);
        }
        for (key, value) in &discovery.headers {
            request = request.header(key, value);
        }
        request = apply_discovery_auth(request, provider.id.as_str(), auth_store);
        let response = request
            .send()
            .with_context(|| format!("failed to fetch models from {url}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(anyhow!(
                "model discovery for {} failed with {status}",
                provider.id
            ));
        }
        let payload = response
            .json::<Value>()
            .with_context(|| format!("failed to parse discovery response from {url}"))?;
        parse_discovered_models(provider, discovery, &payload)
    }
}

/// Merges discovered models into an existing model list by id.
pub fn merge_discovered_models(
    existing: &mut Vec<ModelDescriptor>,
    discovered: Vec<ModelDescriptor>,
) {
    for model in discovered {
        if existing.iter().any(|current| current.id == model.id) {
            continue;
        }
        existing.push(model);
    }
}

fn apply_discovery_auth(
    mut request: reqwest::blocking::RequestBuilder,
    provider_id: &str,
    auth_store: &AuthStore,
) -> reqwest::blocking::RequestBuilder {
    match auth_store.get(provider_id) {
        Some(StoredCredential::ApiKey { key }) if provider_id == "anthropic" => {
            request = request.header("x-api-key", key);
            request = request.header("anthropic-version", "2023-06-01");
            request
        }
        Some(StoredCredential::ApiKey { key }) => {
            request.header("Authorization", format!("Bearer {key}"))
        }
        Some(StoredCredential::OAuth(credential)) => request.header(
            "Authorization",
            format!("Bearer {}", credential.access_token),
        ),
        None => request,
    }
}

fn parse_discovered_models(
    provider: &ProviderDescriptor,
    discovery: &ModelDiscoveryConfig,
    payload: &Value,
) -> Result<Vec<ModelDescriptor>> {
    let items = payload
        .get(discovery.items_field.as_str())
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!(
                "discovery response for {} missing {} array",
                provider.id,
                discovery.items_field
            )
        })?;
    let mut models = Vec::new();
    for item in items {
        let id = item
            .get(discovery.id_field.as_str())
            .and_then(Value::as_str)
            .ok_or_else(|| {
                anyhow!(
                    "discovery model for {} missing {}",
                    provider.id,
                    discovery.id_field
                )
            })?;
        let display_name = match discovery.display_name_field.as_deref() {
            Some(field) => item.get(field).and_then(Value::as_str).unwrap_or(id),
            None => match discovery.response {
                ModelDiscoveryFormat::AnthropicModels => item
                    .get("display_name")
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or(id),
                ModelDiscoveryFormat::OpenAiModels => {
                    item.get("name").and_then(Value::as_str).unwrap_or(id)
                }
            },
        };
        models.push(ModelDescriptor {
            id: id.to_string(),
            display_name: display_name.to_string(),
            provider: provider.id.clone(),
            api: discovery.api.clone(),
            context_window: discovery.context_window,
            max_output_tokens: discovery.max_output_tokens,
            supports_reasoning: discovery.supports_reasoning,
        });
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthMode;
    use indexmap::IndexMap;

    fn provider_with_discovery() -> ProviderDescriptor {
        ProviderDescriptor {
            id: "openai".to_string(),
            display_name: "OpenAI".to_string(),
            base_url: "http://127.0.0.1:1".to_string(),
            default_api: "openai-responses".to_string(),
            auth_modes: vec![AuthMode::ApiKey],
            headers: IndexMap::new(),
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
                headers: IndexMap::new(),
            }),
            models: Vec::new(),
        }
    }

    #[test]
    fn merge_discovered_models_only_adds_missing_ids() {
        let mut models = vec![ModelDescriptor {
            id: "gpt-5".to_string(),
            display_name: "GPT-5".to_string(),
            provider: "openai".to_string(),
            api: "openai-responses".to_string(),
            context_window: 272_000,
            max_output_tokens: 16_384,
            supports_reasoning: true,
        }];
        merge_discovered_models(
            &mut models,
            vec![
                ModelDescriptor {
                    id: "gpt-5".to_string(),
                    display_name: "GPT-5".to_string(),
                    provider: "openai".to_string(),
                    api: "openai-responses".to_string(),
                    context_window: 272_000,
                    max_output_tokens: 16_384,
                    supports_reasoning: true,
                },
                ModelDescriptor {
                    id: "gpt-5-mini".to_string(),
                    display_name: "GPT-5 Mini".to_string(),
                    provider: "openai".to_string(),
                    api: "openai-responses".to_string(),
                    context_window: 272_000,
                    max_output_tokens: 16_384,
                    supports_reasoning: true,
                },
            ],
        );
        assert_eq!(models.len(), 2);
        assert!(models.iter().any(|model| model.id == "gpt-5-mini"));
    }

    #[test]
    fn parse_discovered_models_supports_configured_fields() {
        let mut provider = provider_with_discovery();
        provider.discovery = Some(ModelDiscoveryConfig {
            path: "/models".to_string(),
            response: ModelDiscoveryFormat::AnthropicModels,
            api: "custom-api".to_string(),
            context_window: 8_000,
            max_output_tokens: 2_000,
            supports_reasoning: false,
            items_field: "models".to_string(),
            id_field: "name".to_string(),
            display_name_field: Some("label".to_string()),
            headers: IndexMap::new(),
        });
        let models = parse_discovered_models(
            &provider,
            provider.discovery.as_ref().expect("discovery config"),
            &serde_json::json!({
                "models": [
                    { "name": "custom-a", "label": "Custom A" }
                ]
            }),
        )
        .expect("models");
        assert_eq!(models[0].id, "custom-a");
        assert_eq!(models[0].display_name, "Custom A");
        assert_eq!(models[0].api, "custom-api");
    }

    #[test]
    fn discover_models_without_discovery_returns_empty_list() {
        let client = ModelDiscoveryClient::new();
        let provider = ProviderDescriptor {
            discovery: None,
            ..provider_with_discovery()
        };
        let models = client
            .discover_models(&provider, &AuthStore::default())
            .expect("models");
        assert!(models.is_empty());
    }
}
