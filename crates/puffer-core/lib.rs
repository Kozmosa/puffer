mod command;
mod command_helpers;
mod command_summary;
mod hooks;
mod runtime;
mod state;

pub use command::{dispatch_command, find_command, supported_commands, CommandKind, CommandSpec};
pub(crate) use command_summary::{
    render_buddy_summary, render_cost_summary, render_task_summary, render_usage_summary,
};
pub use hooks::run_resource_hooks;
pub use runtime::execute_user_prompt as execute_user_turn;
pub use runtime::{
    execute_user_prompt_streaming as execute_user_turn_streaming, ToolInvocation, TurnExecution,
    TurnStreamEvent,
};
pub use state::{AppState, MessageRole, RenderedMessage, TaskRecord, TaskStatus};

use anyhow::Result;
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;

/// Reloads declarative resources and rebuilds the provider registry for the active session.
pub fn reload_runtime_resources(
    state: &AppState,
    resources: &mut LoadedResources,
    providers: &mut ProviderRegistry,
    auth_store: &AuthStore,
) -> Result<String> {
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
