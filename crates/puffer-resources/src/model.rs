use puffer_provider_registry::{
    AuthMode, ModelDescriptor, ModelDiscoveryConfig, ProviderDescriptor, ProviderSource,
    ProviderSourceKind,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Identifies which layer produced a resource.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Builtin,
    User,
    Workspace,
}

/// Captures provenance for a resource file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceInfo {
    pub path: PathBuf,
    pub kind: SourceKind,
}

impl SourceInfo {
    /// Converts resource provenance into provider-registry provenance.
    pub fn as_provider_source(&self) -> ProviderSource {
        ProviderSource {
            kind: match self.kind {
                SourceKind::Builtin => ProviderSourceKind::ResourcePack,
                SourceKind::User => ProviderSourceKind::UserConfig,
                SourceKind::Workspace => ProviderSourceKind::WorkspaceConfig,
            },
            path: Some(self.path.display().to_string()),
        }
    }
}

/// Wraps a loaded resource with its source metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadedItem<T> {
    pub value: T,
    pub source_info: SourceInfo,
}

/// Declares a YAML-editable tool specification.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSpec {
    pub id: String,
    pub name: String,
    pub description: String,
    pub handler: String,
    #[serde(default)]
    pub handler_args: Vec<String>,
    #[serde(default)]
    pub approval_policy: Option<String>,
    #[serde(default)]
    pub sandbox_policy: Option<String>,
    #[serde(default)]
    pub shared_lib: Option<String>,
    #[serde(default)]
    pub enabled_if: Option<String>,
    #[serde(default)]
    pub input_schema: Option<Value>,
    #[serde(default)]
    pub metadata: ToolMetadataSpec,
    #[serde(default)]
    pub display: ToolDisplaySpec,
}

/// Declares a YAML-editable prompt template.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptTemplate {
    pub id: String,
    pub description: String,
    pub template: String,
    #[serde(default)]
    pub variables: Vec<PromptVariableSpec>,
    #[serde(default)]
    pub provider_override: Option<String>,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub chained_from: Vec<String>,
}

impl PromptTemplate {
    /// Renders the prompt template using the provided variables and prompt defaults.
    pub fn render(&self, variables: &std::collections::BTreeMap<String, String>) -> String {
        let mut rendered = self.template.clone();
        for variable in &self.variables {
            let key = format!("${}", variable.name);
            let value = variables
                .get(&variable.name)
                .cloned()
                .or_else(|| variable.default.clone())
                .unwrap_or_default();
            rendered = rendered.replace(&key, &value);
        }
        for (name, value) in variables {
            rendered = rendered.replace(&format!("${name}"), value);
        }
        rendered
    }
}

/// Declares one variable accepted by a prompt template.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptVariableSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<String>,
}

/// Carries optional runtime metadata overrides for a declarative tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolMetadataSpec {
    #[serde(default)]
    pub may_spawn_processes: bool,
    #[serde(default)]
    pub may_read_files: bool,
    #[serde(default)]
    pub may_write_files: bool,
}

/// Carries optional display hints for a declarative tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolDisplaySpec {
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub show_in_status: bool,
}

/// Declares a YAML-editable hook specification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookSpec {
    pub id: String,
    pub event: String,
    pub command: String,
}

/// Declares a skill resource loaded from `SKILL.md`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSpec {
    pub name: String,
    pub description: String,
    pub content: String,
    #[serde(default)]
    pub disable_model_invocation: bool,
}

/// Declares a plugin command entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginCommandSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// Declares an MCP server manifest or plugin MCP reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerSpec {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    pub transport: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub description: String,
}

/// Declares a declarative plugin manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginSpec {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub commands: Vec<PluginCommandSpec>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerSpec>,
}

/// Declares a mascot resource.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MascotSpec {
    pub id: String,
    pub display_name: String,
    pub introduction: String,
}

/// Declares an IDE integration manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdeSpec {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
}

/// Declares a provider pack loaded from YAML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderPack {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub default_api: String,
    #[serde(default)]
    pub auth_modes: Vec<AuthMode>,
    #[serde(default)]
    pub headers: indexmap::IndexMap<String, String>,
    #[serde(default)]
    pub query_params: indexmap::IndexMap<String, String>,
    #[serde(default)]
    pub discovery: Option<ModelDiscoveryConfig>,
    #[serde(default)]
    pub models: Vec<ModelDescriptor>,
}

impl ProviderPack {
    /// Converts the provider pack into a registry descriptor.
    pub fn into_descriptor(self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: self.id,
            display_name: self.display_name,
            base_url: self.base_url,
            default_api: self.default_api,
            auth_modes: self.auth_modes,
            headers: self.headers,
            query_params: self.query_params,
            discovery: self.discovery,
            models: self.models,
        }
    }
}

/// Holds all loaded resources across bundled, user, and workspace layers.
#[derive(Debug, Clone, Default)]
pub struct LoadedResources {
    pub providers: Vec<LoadedItem<ProviderPack>>,
    pub tools: Vec<LoadedItem<ToolSpec>>,
    pub prompts: Vec<LoadedItem<PromptTemplate>>,
    pub hooks: Vec<LoadedItem<HookSpec>>,
    pub skills: Vec<LoadedItem<SkillSpec>>,
    pub mascots: Vec<LoadedItem<MascotSpec>>,
    pub plugins: Vec<LoadedItem<PluginSpec>>,
    pub mcp_servers: Vec<LoadedItem<McpServerSpec>>,
    pub ides: Vec<LoadedItem<IdeSpec>>,
    pub diagnostics: Vec<String>,
}
