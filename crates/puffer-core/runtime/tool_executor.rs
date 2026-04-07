use super::agents::execute_agent_tool;
use super::claude_tools::{self, ProviderToolContext};
use crate::AppState;
use anyhow::{anyhow, Result};
use puffer_provider_openai::OpenAIRequestConfig;
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;
use puffer_tools::{ToolExecutionResult, ToolOutput, ToolRegistry};
use puffer_transport_anthropic::AnthropicRequestConfig;
use serde_json::Value;
use std::path::Path;

/// Identifies which provider loop is currently executing a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolExecutionBackend<'a> {
    Anthropic {
        request_config: &'a AnthropicRequestConfig,
    },
    OpenAi {
        request_config: &'a OpenAIRequestConfig,
    },
}

/// Executes one tool call with access to the full conversation runtime context.
pub(super) fn execute_tool_call(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &mut AuthStore,
    registry: &ToolRegistry,
    model_id: &str,
    cwd: &Path,
    backend: ToolExecutionBackend<'_>,
    tool_id: &str,
    input: Value,
) -> Result<ToolExecutionResult> {
    let definition = registry
        .definition(tool_id)
        .ok_or_else(|| anyhow!("unknown tool {tool_id}"))?;
    if definition.handler == "runtime:agent" {
        let output = execute_agent_tool(state, resources, providers, auth_store, cwd, input)?;
        return Ok(successful_runtime_tool(tool_id, output));
    }
    let provider_context = match backend {
        ToolExecutionBackend::Anthropic { request_config } => ProviderToolContext::Anthropic {
            request_config,
            model_id,
        },
        ToolExecutionBackend::OpenAi { request_config } => ProviderToolContext::OpenAI {
            request_config,
            model_id,
        },
    };
    claude_tools::execute_tool(
        state,
        resources,
        registry,
        definition,
        cwd,
        input,
        provider_context,
    )
}

fn successful_runtime_tool(tool_id: &str, stdout: String) -> ToolExecutionResult {
    ToolExecutionResult {
        tool_id: tool_id.to_string(),
        success: true,
        output: ToolOutput {
            stdout,
            stderr: String::new(),
            metadata: Value::Null,
        },
    }
}
