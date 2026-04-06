use crate::auth::AuthMode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Describes the response format used by a provider's model discovery endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelDiscoveryFormat {
    OpenAiModels,
    AnthropicModels,
    OllamaModels,
}

/// Configures runtime discovery for provider-reported models.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelDiscoveryConfig {
    pub path: String,
    pub response: ModelDiscoveryFormat,
    pub api: String,
    pub context_window: u32,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub supports_reasoning: bool,
    #[serde(default = "default_items_field")]
    pub items_field: String,
    #[serde(default = "default_id_field")]
    pub id_field: String,
    #[serde(default)]
    pub display_name_field: Option<String>,
    #[serde(default)]
    pub headers: IndexMap<String, String>,
}

/// Describes the origin of a registered provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSourceKind {
    Builtin,
    ResourcePack,
    UserConfig,
    WorkspaceConfig,
}

/// Carries source provenance for a registered provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderSource {
    pub kind: ProviderSourceKind,
    #[serde(default)]
    pub path: Option<String>,
}

/// Describes one provider model exposed to the rest of the application.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelDescriptor {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub api: String,
    pub context_window: u32,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub supports_reasoning: bool,
}

/// Describes one model provider and the models it exposes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub default_api: String,
    #[serde(default)]
    pub auth_modes: Vec<AuthMode>,
    #[serde(default)]
    pub headers: IndexMap<String, String>,
    #[serde(default)]
    pub query_params: IndexMap<String, String>,
    #[serde(default)]
    pub discovery: Option<ModelDiscoveryConfig>,
    #[serde(default)]
    pub models: Vec<ModelDescriptor>,
}

/// Stores one provider plus its provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisteredProvider {
    pub descriptor: ProviderDescriptor,
    pub source: ProviderSource,
}

fn default_items_field() -> String {
    "data".to_string()
}

fn default_id_field() -> String {
    "id".to_string()
}
