use puffer_connector_core::{ConnectorRuntime, ConversationKey};
use std::sync::Arc;

/// Stable platform id stored in the conversation map.
pub const PLATFORM_ID: &str = "webhook";

/// Outcome of handling one inbound webhook request. The connector uses
/// this to shape the HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    /// Respond with this static text; the agent was not consulted.
    Reply(String),
    /// The agent produced this reply text.
    AgentReply {
        session_id: uuid::Uuid,
        created: bool,
        text: String,
    },
    /// Inbound request was rejected (bad auth / empty body). The
    /// transport translates this into a non-200 response.
    Ignored,
}

/// Pure decision function: given an auth header and a message payload,
/// decide what to send back. Keeping this free of axum types means the
/// full decision tree is unit-testable against an in-memory runtime.
///
/// `auth_header` is the raw value of the `Authorization` header, if
/// present (e.g. `Some("Bearer abc")`). `conversation_id` is whatever
/// the caller supplied in the JSON body.
pub fn handle_command(
    runtime: &Arc<ConnectorRuntime>,
    conversation_id: &str,
    auth_header: Option<&str>,
    text: &str,
    config: &crate::WebhookConfig,
) -> anyhow::Result<CommandOutcome> {
    let presented_token = auth_header.and_then(extract_bearer);
    if !config.is_token_allowed(presented_token.as_deref()) {
        return Ok(CommandOutcome::Ignored);
    }

    let trimmed_id = conversation_id.trim();
    if trimmed_id.is_empty() {
        return Ok(CommandOutcome::Ignored);
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(CommandOutcome::Ignored);
    }

    let key = ConversationKey::new(PLATFORM_ID, trimmed_id);

    if let Some(command) = trimmed.strip_prefix('/') {
        let (name, _args) = command
            .split_once(char::is_whitespace)
            .unwrap_or((command, ""));
        match name.to_ascii_lowercase().as_str() {
            "start" => {
                let greeting = config
                    .welcome_message
                    .clone()
                    .unwrap_or_else(default_welcome);
                return Ok(CommandOutcome::Reply(greeting));
            }
            "new" | "reset" => {
                runtime.reset_conversation(&key)?;
                return Ok(CommandOutcome::Reply(
                    "Started a fresh Puffer session.".to_string(),
                ));
            }
            "help" => {
                return Ok(CommandOutcome::Reply(help_text()));
            }
            _ => {
                // Unknown slash commands fall through to the agent,
                // matching the CLI and Telegram connector behaviour.
            }
        }
    }

    let outcome = runtime.dispatch(&key, trimmed)?;
    Ok(CommandOutcome::AgentReply {
        session_id: outcome.session_id,
        created: outcome.created,
        text: outcome.assistant_text,
    })
}

/// Parses `"Bearer <token>"` (case-insensitive on the scheme) into its
/// token component. Returns `None` for any other shape.
pub(crate) fn extract_bearer(header: &str) -> Option<String> {
    let trimmed = header.trim();
    let (scheme, rest) = trimmed.split_once(char::is_whitespace)?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    let token = rest.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn default_welcome() -> String {
    "Puffer is online. POST JSON { \"conversation_id\": ..., \"message\": ... } \
     or send /help for commands."
        .to_string()
}

fn help_text() -> String {
    "/start — greeting\n\
     /new   — start a fresh session for this conversation_id\n\
     /help  — show this message\n\
     any other text — forwarded to the Puffer agent"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_config::{ConfigPaths, PufferConfig};
    use puffer_connector_core::{
        ConnectorRuntime, ConnectorRuntimeConfig, ConversationSessionMap,
    };
    use puffer_provider_registry::{AuthStore, ProviderRegistry};
    use puffer_resources::LoadedResources;
    use puffer_session_store::SessionStore;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn test_runtime(root: std::path::PathBuf) -> Arc<ConnectorRuntime> {
        let paths = ConfigPaths {
            workspace_root: root.clone(),
            workspace_config_dir: root.join(".puffer"),
            user_config_dir: root.join(".puffer-user"),
            builtin_resources_dir: root.join("resources"),
        };
        std::fs::create_dir_all(&paths.user_config_dir).unwrap();
        Arc::new(ConnectorRuntime::new(ConnectorRuntimeConfig {
            config: PufferConfig::default(),
            resources: LoadedResources::default(),
            providers: ProviderRegistry::default(),
            auth_store: AuthStore::default(),
            auth_path: root.join("auth.json"),
            session_store: SessionStore::from_paths(&paths).unwrap(),
            session_map: ConversationSessionMap::in_memory(),
            default_cwd: root,
        }))
    }

    fn open_config() -> crate::WebhookConfig {
        crate::WebhookConfig {
            bind_address: "127.0.0.1:0".to_string(),
            path: None,
            auth_token: None,
            allowed_origins: Vec::new(),
            welcome_message: None,
        }
    }

    #[test]
    fn wrong_auth_token_is_rejected() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let mut config = open_config();
        config.auth_token = Some("expected".to_string());
        let outcome =
            handle_command(&runtime, "conv-1", Some("Bearer wrong"), "hi", &config).unwrap();
        assert_eq!(outcome, CommandOutcome::Ignored);
    }

    #[test]
    fn empty_message_is_ignored() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let outcome = handle_command(&runtime, "conv-1", None, "   ", &open_config()).unwrap();
        assert_eq!(outcome, CommandOutcome::Ignored);
    }

    #[test]
    fn start_returns_welcome_without_touching_the_agent() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let mut config = open_config();
        config.welcome_message = Some("welcome!".to_string());
        let outcome = handle_command(&runtime, "conv-1", None, "/start", &config).unwrap();
        assert_eq!(outcome, CommandOutcome::Reply("welcome!".to_string()));
    }

    #[test]
    fn start_falls_back_to_default_welcome() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let outcome = handle_command(&runtime, "conv-1", None, "/start", &open_config()).unwrap();
        match outcome {
            CommandOutcome::Reply(text) => assert!(text.contains("Puffer is online")),
            other => panic!("expected Reply, got {other:?}"),
        }
    }

    #[test]
    fn help_command_is_handled_locally() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let outcome = handle_command(&runtime, "conv-1", None, "/help", &open_config()).unwrap();
        match outcome {
            CommandOutcome::Reply(text) => {
                assert!(text.contains("/new"));
                assert!(text.contains("/help"));
            }
            other => panic!("expected Reply, got {other:?}"),
        }
    }

    #[test]
    fn new_command_detaches_the_existing_session() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let key = ConversationKey::new(PLATFORM_ID, "caller-xyz");
        runtime.reset_conversation(&key).unwrap();
        let outcome =
            handle_command(&runtime, "caller-xyz", None, "/new", &open_config()).unwrap();
        assert!(matches!(outcome, CommandOutcome::Reply(_)));
        assert!(runtime.session_for(&key).unwrap().is_none());
    }

    #[test]
    fn valid_auth_allows_local_command() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let mut config = open_config();
        config.auth_token = Some("expected".to_string());
        let outcome =
            handle_command(&runtime, "conv-1", Some("Bearer expected"), "/help", &config).unwrap();
        assert!(matches!(outcome, CommandOutcome::Reply(_)));
    }
}
