use crate::auth::AuthStore;
use crate::discovery::{merge_discovered_models, ModelDiscoveryClient};
use crate::model::{
    ModelDescriptor, ProviderDescriptor, ProviderSource, ProviderSourceKind, RegisteredProvider,
};
use anyhow::{anyhow, Result};
use indexmap::IndexMap;

/// Stores all providers and models known to the application.
#[derive(Debug, Clone, Default)]
pub struct ProviderRegistry {
    providers: IndexMap<String, RegisteredProvider>,
}

impl ProviderRegistry {
    /// Creates an empty provider registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or replaces a provider descriptor using the builtin source kind.
    pub fn register(&mut self, provider: ProviderDescriptor) {
        self.register_with_source(
            provider,
            ProviderSource {
                kind: ProviderSourceKind::Builtin,
                path: None,
            },
        );
    }

    /// Registers or replaces a provider descriptor with explicit provenance.
    pub fn register_with_source(&mut self, provider: ProviderDescriptor, source: ProviderSource) {
        self.providers.insert(
            provider.id.clone(),
            RegisteredProvider {
                descriptor: provider,
                source,
            },
        );
    }

    /// Registers a sequence of providers into the registry.
    pub fn register_many(&mut self, providers: impl IntoIterator<Item = ProviderDescriptor>) {
        for provider in providers {
            self.register(provider);
        }
    }

    /// Returns an iterator over all registered provider descriptors in insertion order.
    pub fn providers(&self) -> impl Iterator<Item = &ProviderDescriptor> {
        self.providers.values().map(|provider| &provider.descriptor)
    }

    /// Returns an iterator over all registered providers including provenance.
    pub fn provider_entries(&self) -> impl Iterator<Item = &RegisteredProvider> {
        self.providers.values()
    }

    /// Looks up a provider descriptor by id.
    pub fn provider(&self, id: &str) -> Option<&ProviderDescriptor> {
        self.providers.get(id).map(|provider| &provider.descriptor)
    }

    /// Looks up a registered provider entry by id.
    pub fn provider_entry(&self, id: &str) -> Option<&RegisteredProvider> {
        self.providers.get(id)
    }

    /// Returns an iterator over all known models across all providers.
    pub fn models(&self) -> impl Iterator<Item = &ModelDescriptor> {
        self.providers
            .values()
            .flat_map(|provider| provider.descriptor.models.iter())
    }

    /// Resolves a model from a `provider/model` selector string.
    pub fn resolve_model(&self, value: &str) -> Option<&ModelDescriptor> {
        let (provider_id, model_id) = value.split_once('/')?;
        self.provider(provider_id)?
            .models
            .iter()
            .find(|model| model.id == model_id)
    }

    /// Discovers and merges runtime models for every provider that exposes discovery config.
    pub fn discover_and_merge_all(&mut self, auth_store: &AuthStore) -> Result<()> {
        let client = ModelDiscoveryClient::new();
        let provider_ids = self.providers.keys().cloned().collect::<Vec<_>>();
        for provider_id in provider_ids {
            self.discover_and_merge_provider_with_client(&provider_id, auth_store, &client)?;
        }
        Ok(())
    }

    /// Discovers and merges runtime models for one provider when discovery is configured.
    pub fn discover_and_merge_provider(
        &mut self,
        provider_id: &str,
        auth_store: &AuthStore,
    ) -> Result<()> {
        let client = ModelDiscoveryClient::new();
        self.discover_and_merge_provider_with_client(provider_id, auth_store, &client)
    }
}

impl ProviderRegistry {
    fn discover_and_merge_provider_with_client(
        &mut self,
        provider_id: &str,
        auth_store: &AuthStore,
        client: &ModelDiscoveryClient,
    ) -> Result<()> {
        let Some(provider) = self.providers.get(provider_id).cloned() else {
            return Err(anyhow!("provider {provider_id} is not registered"));
        };
        if provider.descriptor.discovery.is_none() {
            return Ok(());
        }
        let discovered = client.discover_models(&provider.descriptor, auth_store)?;
        if discovered.is_empty() {
            return Ok(());
        }
        if let Some(entry) = self.providers.get_mut(provider_id) {
            merge_discovered_models(&mut entry.descriptor.models, discovered);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthMode;
    use crate::model::{ModelDiscoveryConfig, ModelDiscoveryFormat};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn provider_descriptor() -> ProviderDescriptor {
        ProviderDescriptor {
            id: "anthropic".to_string(),
            display_name: "Anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            default_api: "anthropic-messages".to_string(),
            auth_modes: vec![AuthMode::ApiKey, AuthMode::OAuth],
            headers: IndexMap::new(),
            discovery: None,
            models: vec![ModelDescriptor {
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

    #[test]
    fn registry_tracks_provider_sources() {
        let mut registry = ProviderRegistry::new();
        registry.register_with_source(
            provider_descriptor(),
            ProviderSource {
                kind: ProviderSourceKind::ResourcePack,
                path: Some("resources/providers/anthropic.yaml".to_string()),
            },
        );

        let entry = registry
            .provider_entry("anthropic")
            .expect("provider entry");
        assert_eq!(entry.source.kind, ProviderSourceKind::ResourcePack);
        assert_eq!(
            registry
                .resolve_model("anthropic/claude-sonnet-4-5")
                .expect("model")
                .display_name,
            "Claude Sonnet 4.5"
        );
    }

    #[test]
    fn parse_openai_discovery_response_maps_models() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || -> String {
            let (mut stream, _) = listener.accept().expect("connection");
            let mut buffer = [0_u8; 4096];
            let size = stream.read(&mut buffer).expect("request");
            let request = String::from_utf8_lossy(&buffer[..size]).to_string();
            let body = serde_json::json!({
                "data": [
                    { "id": "gpt-5" },
                    { "id": "gpt-5-mini" }
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
            request
        });
        let mut provider = provider_descriptor();
        provider.id = "openai".to_string();
        provider.base_url = format!("http://{address}");
        provider.default_api = "openai-responses".to_string();
        provider.discovery = Some(ModelDiscoveryConfig {
            path: "/v1/models".to_string(),
            response: ModelDiscoveryFormat::OpenAiModels,
            api: "openai-responses".to_string(),
            context_window: 272_000,
            max_output_tokens: 16_384,
            supports_reasoning: true,
            items_field: "data".to_string(),
            id_field: "id".to_string(),
            display_name_field: None,
            headers: IndexMap::from([("x-test-header".to_string(), "present".to_string())]),
        });
        let mut auth = AuthStore::default();
        auth.set_api_key("openai", "sk-openai");
        let mut registry = ProviderRegistry::new();
        registry.register(provider);

        registry
            .discover_and_merge_provider("openai", &auth)
            .expect("discovery succeeds");

        let request = server.join().expect("server thread");
        let entry = registry.provider("openai").expect("provider");
        assert!(entry.models.iter().any(|model| model.id == "gpt-5"));
        assert!(entry.models.iter().any(|model| model.id == "gpt-5-mini"));
        assert!(request.contains("GET /v1/models HTTP/1.1"));
        assert!(request.contains("authorization: Bearer sk-openai"));
        assert!(request.contains("x-test-header: present"));
    }
}
