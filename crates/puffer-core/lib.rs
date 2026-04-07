mod agent_catalog;
mod command;
mod command_helpers;
mod command_summary;
mod hooks;
mod permissions;
mod runtime;
mod state;

pub use agent_catalog::{load_agent_catalog, AgentCatalogEntry};
pub use command::{dispatch_command, find_command, supported_commands, CommandKind, CommandSpec};
pub use command_helpers::McpActionEntry;
pub use command_helpers::PluginActionEntry;
pub use command_helpers::SessionOverlayView;
pub use command_helpers::TaskActionEntry;
pub(crate) use command_summary::{render_buddy_summary, render_cost_summary, render_usage_summary};
pub use hooks::run_resource_hooks;
pub use runtime::execute_user_prompt as execute_user_turn;
pub use runtime::{
    execute_side_question, execute_user_prompt_streaming as execute_user_turn_streaming,
    execute_user_prompt_streaming_with_permissions as execute_user_turn_streaming_with_permissions,
    execute_user_prompt_streaming_with_structured_output as execute_user_turn_streaming_with_structured_output,
    execute_user_prompt_with_structured_output as execute_user_turn_with_structured_output,
    shutdown_runtime_services, with_permission_prompt_handler, PermissionPromptAction,
    PermissionPromptRequest, StructuredOutputConfig, ToolInvocation, TurnExecution,
    TurnStreamEvent,
};
pub use state::{AppState, MessageRole, RenderedMessage, TaskRecord, TaskStatus};

use anyhow::Result;
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;

/// Renders the current session/provider/tool status summary used by `/status`.
pub fn render_status_summary(
    state: &AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
) -> String {
    command_summary::render_status_summary(state, resources, providers, auth_store)
}

/// Renders the current `/config` summary used by interactive overlays.
pub fn render_config_summary(state: &AppState) -> Result<String> {
    command_helpers::render_config_summary(state)
}

/// Renders the current `/permissions` summary used by interactive overlays.
pub fn render_permissions_panel(state: &AppState, resources: &LoadedResources) -> Result<String> {
    command_helpers::render_permissions_panel(state, resources)
}

/// Renders the current `/hooks` summary used by interactive overlays.
pub fn render_hooks_summary(state: &AppState, resources: &LoadedResources) -> Result<String> {
    command_helpers::render_hooks_summary(state, resources)
}

/// Renders the grouped `/skills` summary used by the interactive overlay.
pub fn render_skills_panel(resources: &LoadedResources) -> String {
    command_helpers::render_skills_panel(resources)
}

/// Renders the current `/mcp` summary used by interactive overlays.
pub fn render_mcp_summary(state: &AppState, resources: &LoadedResources) -> Result<String> {
    command_helpers::render_mcp_summary(state, resources)
}

/// Builds the interactive `/mcp` action list used by the TUI picker.
pub fn render_mcp_actions(
    state: &AppState,
    resources: &LoadedResources,
) -> Result<Vec<McpActionEntry>> {
    command_helpers::render_mcp_actions(state, resources)
}

/// Renders the current `/plugin` summary used by interactive overlays.
pub fn render_plugin_summary(state: &AppState, resources: &LoadedResources) -> Result<String> {
    command_helpers::render_plugin_summary(state, resources)
}

/// Builds the interactive `/plugin` action list used by the TUI picker.
pub fn render_plugin_actions(
    state: &AppState,
    resources: &LoadedResources,
) -> Result<Vec<PluginActionEntry>> {
    command_helpers::render_plugin_actions(state, resources)
}

/// Builds the interactive `/tasks` action list used by the TUI picker.
pub fn render_task_actions(state: &mut AppState) -> Result<Vec<TaskActionEntry>> {
    command_helpers::render_task_actions(state)
}

/// Renders the current `/memory` summary used by interactive overlays.
pub fn render_memory_panel(state: &AppState) -> String {
    command_helpers::render_memory_panel(state)
}

/// Renders the current `/session` summary used by interactive overlays.
pub fn render_session_panel(state: &AppState) -> String {
    command_helpers::render_session_panel(state)
}

/// Builds the dedicated `/session` overlay view used by the interactive TUI.
pub fn render_session_overlay(state: &AppState) -> SessionOverlayView {
    command_helpers::render_session_overlay(state)
}

/// Reloads declarative resources and rebuilds the provider registry for the active session.
pub fn reload_runtime_resources(
    state: &AppState,
    resources: &mut LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &AuthStore,
) -> Result<String> {
    let _ = shutdown_runtime_services();
    *resources = command_helpers::reload_resources_from_disk(state)?;
    *providers = ProviderRegistry::new();
    for provider in &resources.providers {
        providers.register_with_source(
            provider.value.clone().into_descriptor(),
            provider.source_info.as_provider_source(),
        );
    }
    let _ = providers.discover_and_merge_all(auth_store);
    command_helpers::reload_plugins_summary(state, resources)
}
