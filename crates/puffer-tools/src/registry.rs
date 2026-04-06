use crate::{execute_builtin_tool, ToolExecutionResult, ToolInput, ToolKind};
use anyhow::{anyhow, Result};
use puffer_resources::LoadedResources;
use puffer_resources::ToolSpec;
use std::collections::BTreeMap;
use std::path::Path;

/// One registered tool with its declarative metadata and runtime kind.
#[derive(Debug, Clone)]
pub struct RegisteredTool {
    pub spec: ToolSpec,
    pub kind: ToolKind,
}

/// Registry of executable built-in tools derived from loaded resources.
#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, RegisteredTool>,
}

impl ToolRegistry {
    /// Builds a tool registry from loaded declarative resources.
    pub fn from_resources(resources: &LoadedResources) -> Self {
        let mut registry = Self::default();
        for item in &resources.tools {
            if let Some(kind) = handler_to_kind(&item.value.handler) {
                registry.tools.insert(
                    item.value.id.clone(),
                    RegisteredTool {
                        spec: item.value.clone(),
                        kind,
                    },
                );
            }
        }
        registry
    }

    /// Returns all registered tools in stable id order.
    pub fn tools(&self) -> impl Iterator<Item = &RegisteredTool> {
        self.tools.values()
    }

    /// Looks up a registered tool by id.
    pub fn tool(&self, tool_id: &str) -> Option<&RegisteredTool> {
        self.tools.get(tool_id)
    }

    /// Executes a registered tool by id with typed input.
    pub fn execute(
        &self,
        tool_id: &str,
        cwd: &Path,
        input: ToolInput,
    ) -> Result<ToolExecutionResult> {
        let tool = self
            .tool(tool_id)
            .ok_or_else(|| anyhow!("unknown tool {tool_id}"))?;
        execute_builtin_tool(tool_id, tool.kind, cwd, input)
    }

    /// Executes a registered tool by id using a raw JSON input payload.
    pub fn execute_json(
        &self,
        tool_id: &str,
        cwd: &Path,
        input: serde_json::Value,
    ) -> Result<ToolExecutionResult> {
        let tool = self
            .tool(tool_id)
            .ok_or_else(|| anyhow!("unknown tool {tool_id}"))?;
        let typed = parse_input(tool.kind, input)?;
        execute_builtin_tool(tool_id, tool.kind, cwd, typed)
    }
}

fn handler_to_kind(handler: &str) -> Option<ToolKind> {
    match handler {
        "bash" => Some(ToolKind::Bash),
        "read_file" => Some(ToolKind::ReadFile),
        "write_file" => Some(ToolKind::WriteFile),
        _ => None,
    }
}

fn parse_input(kind: ToolKind, input: serde_json::Value) -> Result<ToolInput> {
    match kind {
        ToolKind::Bash => Ok(ToolInput::Bash(serde_json::from_value(input)?)),
        ToolKind::ReadFile => Ok(ToolInput::ReadFile(serde_json::from_value(input)?)),
        ToolKind::WriteFile => Ok(ToolInput::WriteFile(serde_json::from_value(input)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_resources::{LoadedItem, LoadedResources, SourceInfo, SourceKind, ToolSpec};
    use std::path::PathBuf;

    #[test]
    fn registry_builds_from_resources() {
        let resources = LoadedResources {
            tools: vec![LoadedItem {
                value: ToolSpec {
                    id: "bash".to_string(),
                    name: "bash".to_string(),
                    description: "Run shell".to_string(),
                    handler: "bash".to_string(),
                    approval_policy: Some("on-request".to_string()),
                    sandbox_policy: Some("workspace-write".to_string()),
                },
                source_info: SourceInfo {
                    path: PathBuf::from("bash.yaml"),
                    kind: SourceKind::Builtin,
                },
            }],
            ..LoadedResources::default()
        };
        let registry = ToolRegistry::from_resources(&resources);
        assert!(registry.tool("bash").is_some());
    }

    #[test]
    fn execute_json_parses_input_for_registered_tool() {
        let resources = LoadedResources {
            tools: vec![LoadedItem {
                value: ToolSpec {
                    id: "bash".to_string(),
                    name: "bash".to_string(),
                    description: "Run shell".to_string(),
                    handler: "bash".to_string(),
                    approval_policy: Some("on-request".to_string()),
                    sandbox_policy: Some("workspace-write".to_string()),
                },
                source_info: SourceInfo {
                    path: PathBuf::from("bash.yaml"),
                    kind: SourceKind::Builtin,
                },
            }],
            ..LoadedResources::default()
        };
        let registry = ToolRegistry::from_resources(&resources);
        let temp = tempfile::tempdir().unwrap();
        let result = registry
            .execute_json(
                "bash",
                temp.path(),
                serde_json::json!({ "command": "printf hi" }),
            )
            .unwrap();
        assert_eq!(result.output.stdout, "hi");
    }
}
