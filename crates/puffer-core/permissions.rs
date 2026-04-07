use crate::AppState;
use anyhow::{bail, Result};
use puffer_config::ConfigPaths;
use puffer_resources::LoadedResources;
use puffer_tools::ToolDefinition;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

/// Stores persisted workspace permission overrides for tool ids.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct PermissionsSettings {
    #[serde(default)]
    pub(crate) tools: BTreeMap<String, String>,
}

/// Stores persisted workspace sandbox preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SandboxSettings {
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) auto_allow: bool,
    #[serde(default)]
    pub(crate) allow_unsandboxed_fallback: bool,
    #[serde(default)]
    pub(crate) excluded_commands: Vec<String>,
}

impl SandboxSettings {
    /// Builds the default sandbox settings for the active session.
    pub(crate) fn from_mode(mode: &str) -> Self {
        Self {
            mode: mode.to_string(),
            auto_allow: false,
            allow_unsandboxed_fallback: false,
            excluded_commands: Vec::new(),
        }
    }
}

/// Describes how the runtime should handle one tool invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolPermissionBehavior {
    Allow,
    Ask,
    Deny,
}

/// Carries the chosen permission behavior plus an optional explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolPermissionDecision {
    pub(crate) behavior: ToolPermissionBehavior,
    pub(crate) reason: Option<String>,
}

/// Carries the effective runtime permission state for one model turn.
#[derive(Debug, Clone)]
pub(crate) struct RuntimePermissionContext {
    permissions: PermissionsSettings,
    sandbox: SandboxSettings,
    plan_mode: bool,
}

impl RuntimePermissionContext {
    /// Returns true when the tool should stay visible in the provider tool list.
    pub(crate) fn tool_visible_to_model(&self, definition: &ToolDefinition) -> bool {
        if tool_skips_permission_enforcement(definition) {
            return true;
        }
        self.decision_for_tool_call(definition, &Value::Null)
            .behavior
            != ToolPermissionBehavior::Deny
    }

    /// Computes the effective permission decision for one tool invocation.
    pub(crate) fn decision_for_tool_call(
        &self,
        definition: &ToolDefinition,
        input: &Value,
    ) -> ToolPermissionDecision {
        if tool_skips_permission_enforcement(definition) {
            return ToolPermissionDecision {
                behavior: ToolPermissionBehavior::Allow,
                reason: None,
            };
        }
        if definition
            .policy
            .approval_policy
            .as_deref()
            .is_some_and(policy_value_disables_tool)
        {
            return ToolPermissionDecision {
                behavior: ToolPermissionBehavior::Deny,
                reason: Some("tool metadata marks it disabled".to_string()),
            };
        }
        if definition
            .enabled_if
            .as_deref()
            .is_some_and(enabled_if_value_disables_tool)
        {
            return ToolPermissionDecision {
                behavior: ToolPermissionBehavior::Deny,
                reason: Some("tool metadata currently disables it".to_string()),
            };
        }
        if let Some(policy) = self
            .permissions
            .tools
            .get(&normalize_tool_id(&definition.id))
            .map(String::as_str)
        {
            return self.policy_decision(definition, input, policy);
        }

        if let Some(decision) = self.tool_specific_decision(definition, input) {
            return decision;
        }

        let policy = definition.policy.approval_policy.as_deref().unwrap_or("auto");
        self.policy_decision(definition, input, policy)
    }

    fn policy_decision(
        &self,
        definition: &ToolDefinition,
        input: &Value,
        policy: &str,
    ) -> ToolPermissionDecision {
        match normalize_policy_value(policy).as_str() {
            "deny" | "disabled" => ToolPermissionDecision {
                behavior: ToolPermissionBehavior::Deny,
                reason: Some("workspace permission rule set this tool to deny".to_string()),
            },
            "ask" => ToolPermissionDecision {
                behavior: ToolPermissionBehavior::Ask,
                reason: Some("workspace permission rule requires approval".to_string()),
            },
            "on-request" => {
                if let Some(reason) = self.approval_reason(definition, input) {
                    ToolPermissionDecision {
                        behavior: ToolPermissionBehavior::Ask,
                        reason: Some(reason),
                    }
                } else {
                    ToolPermissionDecision {
                        behavior: ToolPermissionBehavior::Allow,
                        reason: None,
                    }
                }
            }
            _ => ToolPermissionDecision {
                behavior: ToolPermissionBehavior::Allow,
                reason: None,
            },
        }
    }

    fn tool_specific_decision(
        &self,
        definition: &ToolDefinition,
        input: &Value,
    ) -> Option<ToolPermissionDecision> {
        match definition.id.as_str() {
            "Config" => Some(if input.get("value").is_some() {
                ToolPermissionDecision {
                    behavior: ToolPermissionBehavior::Ask,
                    reason: Some("config writes require approval".to_string()),
                }
            } else {
                ToolPermissionDecision {
                    behavior: ToolPermissionBehavior::Allow,
                    reason: None,
                }
            }),
            "AskUserQuestion" => Some(ToolPermissionDecision {
                behavior: ToolPermissionBehavior::Ask,
                reason: Some("answer questions?".to_string()),
            }),
            "WebSearch" => Some(ToolPermissionDecision {
                behavior: ToolPermissionBehavior::Ask,
                reason: Some("web search requires permission".to_string()),
            }),
            "SendMessage" => {
                let target = input.get("to").and_then(Value::as_str).unwrap_or_default();
                if target.starts_with("bridge:") {
                    Some(ToolPermissionDecision {
                        behavior: ToolPermissionBehavior::Ask,
                        reason: Some(
                            "cross-session bridge messages require explicit approval".to_string(),
                        ),
                    })
                } else {
                    Some(ToolPermissionDecision {
                        behavior: ToolPermissionBehavior::Allow,
                        reason: None,
                    })
                }
            }
            "TodoWrite" => Some(ToolPermissionDecision {
                behavior: ToolPermissionBehavior::Allow,
                reason: None,
            }),
            "Agent" => Some(ToolPermissionDecision {
                behavior: ToolPermissionBehavior::Allow,
                reason: None,
            }),
            _ => None,
        }
    }

    /// Enforces the effective permission decision for one tool invocation.
    pub(crate) fn enforce_tool_call(
        &self,
        definition: &ToolDefinition,
        input: &Value,
    ) -> Result<()> {
        let decision = self.decision_for_tool_call(definition, input);
        match decision.behavior {
            ToolPermissionBehavior::Allow => Ok(()),
            ToolPermissionBehavior::Deny => bail!(
                "tool `{}` is denied by permission policy: {}",
                definition.id,
                decision
                    .reason
                    .unwrap_or_else(|| "workspace rule denied it".to_string())
            ),
            ToolPermissionBehavior::Ask => {
                let mut message = format!(
                    "tool `{}` requires approval before execution",
                    definition.id
                );
                if let Some(reason) = decision.reason {
                    let _ = write!(&mut message, ": {reason}");
                }
                let _ = write!(
                    &mut message,
                    ". Use `/permissions allow {}` to allow it for this workspace.",
                    definition.id
                );
                if shell_requests_unsandboxed(definition, input) {
                    message.push_str(
                        " If you intended to bypass sandboxing, enable `/sandbox allow-unsandboxed true` first.",
                    );
                }
                bail!(message)
            }
        }
    }

    fn approval_reason(&self, definition: &ToolDefinition, input: &Value) -> Option<String> {
        if let Some(reason) = shell_sandbox_reason(definition, input, &self.sandbox) {
            return Some(reason);
        }
        if self.plan_mode && tool_mutates_workspace(definition) {
            return Some(format!(
                "plan mode requires approval for mutating tools. Use `ExitPlanMode` before retrying `{}`.",
                definition.id
            ));
        }
        None
    }
}

/// Normalizes a tool id so workspace settings can key tools consistently.
pub(crate) fn normalize_tool_id(tool: &str) -> String {
    tool.trim().replace('-', "_").to_ascii_lowercase()
}

/// Renders the default permissions file contents for the loaded tool surface.
pub(crate) fn default_permissions_contents(resources: &LoadedResources) -> String {
    let mut text = String::from("[tools]\n");
    for tool in &resources.tools {
        let key = normalize_tool_id(&tool.value.id);
        let value = tool
            .value
            .approval_policy
            .as_deref()
            .unwrap_or("auto")
            .trim();
        let value = if value.is_empty() { "auto" } else { value };
        let _ = writeln!(&mut text, "{key} = \"{value}\"");
    }
    if resources.tools.is_empty() {
        text.push_str("bash = \"on-request\"\n");
    }
    text
}

/// Loads or initializes the workspace permissions file.
pub(crate) fn load_or_initialize_permissions(
    path: &Path,
    resources: &LoadedResources,
) -> Result<PermissionsSettings> {
    if path.exists() {
        return load_permissions_settings(path);
    }
    fs::write(path, default_permissions_contents(resources))?;
    load_permissions_settings(path)
}

/// Loads the sandbox settings for runtime evaluation without creating files on disk.
pub(crate) fn load_runtime_sandbox_settings(
    cwd: &Path,
    state: &AppState,
) -> Result<SandboxSettings> {
    let paths = ConfigPaths::discover(cwd);
    let sandbox_path = paths.workspace_config_dir.join("sandbox.toml");
    if sandbox_path.exists() {
        return Ok(toml::from_str(&fs::read_to_string(sandbox_path)?)?);
    }
    Ok(SandboxSettings::from_mode(&state.sandbox_mode))
}

/// Loads or initializes the workspace sandbox settings file.
pub(crate) fn load_or_initialize_sandbox_settings(
    path: &Path,
    state: &AppState,
) -> Result<SandboxSettings> {
    if path.exists() {
        return Ok(toml::from_str(&fs::read_to_string(path)?)?);
    }
    let settings = SandboxSettings::from_mode(&state.sandbox_mode);
    write_sandbox_settings(path, &settings)?;
    Ok(settings)
}

/// Loads the effective permission context for one model turn or tool invocation.
pub(crate) fn load_runtime_permission_context(
    cwd: &Path,
    _resources: &LoadedResources,
    state: &AppState,
) -> Result<RuntimePermissionContext> {
    let paths = ConfigPaths::discover(cwd);
    let permissions_path = paths.workspace_config_dir.join("permissions.toml");
    let mut permissions = if permissions_path.exists() {
        load_permissions_settings(&permissions_path)?
    } else {
        PermissionsSettings::default()
    };
    permissions
        .tools
        .extend(state.session_tool_permissions.clone());
    Ok(RuntimePermissionContext {
        permissions,
        sandbox: load_runtime_sandbox_settings(cwd, state)?,
        plan_mode: state.plan_mode,
    })
}

/// Writes the permissions file to disk.
pub(crate) fn write_permissions(path: &Path, settings: &PermissionsSettings) -> Result<()> {
    fs::write(path, toml::to_string_pretty(settings)?)?;
    Ok(())
}

/// Writes the sandbox settings file to disk.
pub(crate) fn write_sandbox_settings(path: &Path, settings: &SandboxSettings) -> Result<()> {
    fs::write(path, toml::to_string_pretty(settings)?)?;
    Ok(())
}

fn normalize_policy_value(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn load_permissions_settings(path: &Path) -> Result<PermissionsSettings> {
    let loaded: PermissionsSettings = toml::from_str(&fs::read_to_string(path)?)?;
    Ok(PermissionsSettings {
        tools: loaded
            .tools
            .into_iter()
            .map(|(tool, level)| (normalize_tool_id(&tool), normalize_policy_value(&level)))
            .collect(),
    })
}

fn policy_value_disables_tool(value: &str) -> bool {
    matches!(normalize_policy_value(value).as_str(), "disabled" | "deny")
}

fn enabled_if_value_disables_tool(value: &str) -> bool {
    matches!(
        normalize_policy_value(value).as_str(),
        "0" | "disabled" | "deny" | "false" | "never" | "off"
    )
}

fn tool_mutates_workspace(definition: &ToolDefinition) -> bool {
    definition.metadata.may_write_files
        || definition.metadata.may_spawn_processes
        || definition.policy.sandbox_policy.as_deref() == Some("workspace-write")
}

fn tool_skips_permission_enforcement(definition: &ToolDefinition) -> bool {
    matches!(definition.id.as_str(), "SendUserMessage" | "Brief")
}

fn shell_requests_unsandboxed(definition: &ToolDefinition, input: &Value) -> bool {
    matches!(definition.id.as_str(), "Bash" | "PowerShell")
        && input
            .get("dangerouslyDisableSandbox")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn shell_sandbox_reason(
    definition: &ToolDefinition,
    input: &Value,
    sandbox: &SandboxSettings,
) -> Option<String> {
    let command = input.get("command").and_then(Value::as_str)?;
    if let Some(pattern) = sandbox
        .excluded_commands
        .iter()
        .find(|pattern| !pattern.trim().is_empty() && command.contains(pattern.as_str()))
    {
        if sandbox.allow_unsandboxed_fallback {
            return None;
        }
        return Some(format!(
            "shell command matches sandbox exclusion `{}`",
            pattern.trim()
        ));
    }
    if shell_requests_unsandboxed(definition, input) && !sandbox.allow_unsandboxed_fallback {
        return Some(
            "shell command requested dangerouslyDisableSandbox without unsandboxed fallback enabled"
                .to_string(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_resources::{LoadedItem, SourceInfo, SourceKind, ToolSpec};

    fn tool_definition(id: &str, approval_policy: &str) -> ToolDefinition {
        ToolDefinition {
            id: id.to_string(),
            name: id.to_string(),
            description: id.to_string(),
            handler: id.to_string(),
            handler_args: Vec::new(),
            kind: puffer_tools::ToolKind::Custom,
            input_schema: puffer_tools::ToolInputSchema::default(),
            metadata: puffer_tools::ToolMetadata {
                may_spawn_processes: id == "Bash" || id == "PowerShell",
                may_read_files: false,
                may_write_files: id == "Write",
            },
            policy: puffer_tools::ToolPolicyHints {
                approval_policy: Some(approval_policy.to_string()),
                sandbox_policy: Some("workspace-write".to_string()),
            },
            shared_lib: None,
            enabled_if: None,
            display: puffer_tools::ToolDisplayHints::default(),
        }
    }

    #[test]
    fn default_permissions_contents_follow_declared_policy() {
        let contents = default_permissions_contents(&LoadedResources {
            tools: vec![
                LoadedItem {
                    value: ToolSpec {
                        id: "Bash".to_string(),
                        name: "Bash".to_string(),
                        description: "Bash".to_string(),
                        handler: "bash".to_string(),
                        handler_args: Vec::new(),
                        approval_policy: Some("on-request".to_string()),
                        sandbox_policy: None,
                        shared_lib: None,
                        enabled_if: None,
                        input_schema: None,
                        metadata: Default::default(),
                        display: Default::default(),
                    },
                    source_info: SourceInfo {
                        path: "bash.yaml".into(),
                        kind: SourceKind::Builtin,
                    },
                },
                LoadedItem {
                    value: ToolSpec {
                        id: "Read".to_string(),
                        name: "Read".to_string(),
                        description: "Read".to_string(),
                        handler: "read".to_string(),
                        handler_args: Vec::new(),
                        approval_policy: Some("auto".to_string()),
                        sandbox_policy: None,
                        shared_lib: None,
                        enabled_if: None,
                        input_schema: None,
                        metadata: Default::default(),
                        display: Default::default(),
                    },
                    source_info: SourceInfo {
                        path: "read.yaml".into(),
                        kind: SourceKind::Builtin,
                    },
                },
            ],
            ..LoadedResources::default()
        });
        assert!(contents.contains("bash = \"on-request\""));
        assert!(contents.contains("read = \"auto\""));
    }

    #[test]
    fn plan_mode_marks_mutating_on_request_tools_as_ask() {
        let context = RuntimePermissionContext {
            permissions: PermissionsSettings::default(),
            sandbox: SandboxSettings::from_mode("workspace-write"),
            plan_mode: true,
        };
        let decision =
            context.decision_for_tool_call(&tool_definition("Write", "on-request"), &Value::Null);
        assert_eq!(decision.behavior, ToolPermissionBehavior::Ask);
        assert!(decision.reason.unwrap_or_default().contains("ExitPlanMode"));
    }

    #[test]
    fn config_reads_allow_but_writes_ask() {
        let context = RuntimePermissionContext {
            permissions: PermissionsSettings::default(),
            sandbox: SandboxSettings::from_mode("workspace-write"),
            plan_mode: false,
        };
        let config = tool_definition("Config", "auto");
        let read = context.decision_for_tool_call(&config, &serde_json::json!({"setting":"theme"}));
        let write = context.decision_for_tool_call(
            &config,
            &serde_json::json!({"setting":"theme","value":"dark"}),
        );
        assert_eq!(read.behavior, ToolPermissionBehavior::Allow);
        assert_eq!(write.behavior, ToolPermissionBehavior::Ask);
    }

    #[test]
    fn ask_user_question_requires_approval() {
        let context = RuntimePermissionContext {
            permissions: PermissionsSettings::default(),
            sandbox: SandboxSettings::from_mode("workspace-write"),
            plan_mode: false,
        };
        let question = tool_definition("AskUserQuestion", "auto");
        let decision = context.decision_for_tool_call(
            &question,
            &serde_json::json!({"questions":[{"question":"Pick one","header":"Choice","options":[{"label":"A","description":"A"},{"label":"B","description":"B"}]}]}),
        );
        assert_eq!(decision.behavior, ToolPermissionBehavior::Ask);
    }

    #[test]
    fn web_search_requires_permission_by_default() {
        let context = RuntimePermissionContext {
            permissions: PermissionsSettings::default(),
            sandbox: SandboxSettings::from_mode("workspace-write"),
            plan_mode: false,
        };
        let search = tool_definition("WebSearch", "auto");
        let decision =
            context.decision_for_tool_call(&search, &serde_json::json!({"query":"rust latest"}));
        assert_eq!(decision.behavior, ToolPermissionBehavior::Ask);
    }

    #[test]
    fn send_message_allows_local_targets_but_asks_for_bridge_targets() {
        let context = RuntimePermissionContext {
            permissions: PermissionsSettings::default(),
            sandbox: SandboxSettings::from_mode("workspace-write"),
            plan_mode: false,
        };
        let send = tool_definition("SendMessage", "auto");
        let local =
            context.decision_for_tool_call(&send, &serde_json::json!({"to":"alice","message":"hi"}));
        let bridge = context.decision_for_tool_call(
            &send,
            &serde_json::json!({"to":"bridge:session-123","message":"hi"}),
        );
        assert_eq!(local.behavior, ToolPermissionBehavior::Allow);
        assert_eq!(bridge.behavior, ToolPermissionBehavior::Ask);
    }

    #[test]
    fn todo_write_and_agent_are_allowed_without_extra_gate() {
        let context = RuntimePermissionContext {
            permissions: PermissionsSettings::default(),
            sandbox: SandboxSettings::from_mode("workspace-write"),
            plan_mode: true,
        };
        let todo = tool_definition("TodoWrite", "auto");
        let agent = tool_definition("Agent", "auto");
        let todo_decision = context.decision_for_tool_call(
            &todo,
            &serde_json::json!({"todos":[{"content":"x","status":"pending","activeForm":"Doing x"}]}),
        );
        let agent_decision = context.decision_for_tool_call(
            &agent,
            &serde_json::json!({"description":"Task","prompt":"Do it"}),
        );
        assert_eq!(todo_decision.behavior, ToolPermissionBehavior::Allow);
        assert_eq!(agent_decision.behavior, ToolPermissionBehavior::Allow);
    }

    #[test]
    fn disabled_tool_is_hidden_from_model_pool() {
        let mut definition = tool_definition("Bash", "on-request");
        definition.policy.approval_policy = Some("disabled".to_string());
        let context = RuntimePermissionContext {
            permissions: PermissionsSettings::default(),
            sandbox: SandboxSettings::from_mode("workspace-write"),
            plan_mode: false,
        };
        assert!(!context.tool_visible_to_model(&definition));
    }

    #[test]
    fn send_user_message_ignores_workspace_ask_rules() {
        let context = RuntimePermissionContext {
            permissions: PermissionsSettings {
                tools: BTreeMap::from([
                    ("sendusermessage".to_string(), "ask".to_string()),
                    ("brief".to_string(), "deny".to_string()),
                ]),
            },
            sandbox: SandboxSettings::from_mode("workspace-write"),
            plan_mode: true,
        };
        let send_user_message = ToolDefinition {
            id: "SendUserMessage".to_string(),
            name: "SendUserMessage".to_string(),
            description: String::new(),
            handler: "runtime:workflow:send_user_message".to_string(),
            handler_args: Vec::new(),
            kind: puffer_tools::ToolKind::Custom,
            input_schema: puffer_tools::ToolInputSchema::default(),
            metadata: puffer_tools::ToolMetadata::default(),
            policy: puffer_tools::ToolPolicyHints {
                approval_policy: Some("auto".to_string()),
                sandbox_policy: Some("read-only".to_string()),
            },
            shared_lib: None,
            enabled_if: None,
            display: puffer_tools::ToolDisplayHints::default(),
        };
        let brief = ToolDefinition {
            id: "Brief".to_string(),
            ..send_user_message.clone()
        };

        let send_decision =
            context.decision_for_tool_call(&send_user_message, &serde_json::json!({"message": "hi"}));
        let brief_decision =
            context.decision_for_tool_call(&brief, &serde_json::json!({"message": "hi"}));

        assert_eq!(send_decision.behavior, ToolPermissionBehavior::Allow);
        assert_eq!(brief_decision.behavior, ToolPermissionBehavior::Allow);
        assert!(context.tool_visible_to_model(&send_user_message));
        assert!(context.tool_visible_to_model(&brief));
    }
}
