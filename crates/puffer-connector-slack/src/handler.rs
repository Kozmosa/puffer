use puffer_connector_core::{ConnectorRuntime, ConversationKey};
use std::sync::Arc;

/// Stable platform id stored in the conversation map.
pub(crate) const PLATFORM_ID: &str = "slack";

/// Outcome of handling one inbound text message. The connector uses this
/// to decide what to send back on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    /// Reply with this static text; the agent was not consulted.
    Reply(String),
    /// The agent produced this reply text.
    AgentReply {
        session_id: uuid::Uuid,
        created: bool,
        text: String,
    },
    /// The inbound user is not permitted to talk to the bot. Silently
    /// ignore.
    Ignored,
}

/// Builds the ConversationKey used by the Slack connector.
///
/// When `thread_ts` is `Some` the key disambiguates threads in the same
/// channel so one Slack channel can host multiple parallel Puffer
/// conversations.
pub fn conversation_key(channel_id: &str, thread_ts: Option<&str>) -> ConversationKey {
    let conversation = match thread_ts {
        Some(ts) if !ts.is_empty() => format!("{channel_id}:{ts}"),
        _ => channel_id.to_string(),
    };
    ConversationKey::new(PLATFORM_ID, conversation)
}

/// Dispatches one inbound message. Pure logic — no Slack SDK calls here —
/// so the full decision tree is unit-testable against an in-memory
/// runtime.
pub fn handle_command(
    runtime: &Arc<ConnectorRuntime>,
    channel_id: &str,
    user_id: Option<&str>,
    text: &str,
    config: &crate::SlackConfig,
) -> anyhow::Result<CommandOutcome> {
    handle_command_threaded(runtime, channel_id, None, user_id, text, config)
}

/// Variant of [`handle_command`] that accepts an optional Slack
/// `thread_ts` so threaded replies map to their own Puffer session.
pub fn handle_command_threaded(
    runtime: &Arc<ConnectorRuntime>,
    channel_id: &str,
    thread_ts: Option<&str>,
    user_id: Option<&str>,
    text: &str,
    config: &crate::SlackConfig,
) -> anyhow::Result<CommandOutcome> {
    if let Some(id) = user_id {
        if !config.is_user_allowed(id) {
            return Ok(CommandOutcome::Ignored);
        }
    }

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(CommandOutcome::Ignored);
    }

    let key = conversation_key(channel_id, thread_ts);

    // Slash commands are handled locally and never reach the agent.
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
                // Fall through — unknown commands become plain text the
                // agent can interpret (consistent with Puffer CLI which
                // forwards unknown /slash commands).
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

fn default_welcome() -> String {
    "Puffer is online. Send any message to talk to the agent, or /help \
     for commands."
        .to_string()
}

fn help_text() -> String {
    "/start — greeting\n\
     /new   — start a fresh session for this channel/thread\n\
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

    fn open_config() -> crate::SlackConfig {
        crate::SlackConfig {
            bot_token: "xoxb-t".to_string(),
            app_token: "xapp-t".to_string(),
            allowed_users: Vec::new(),
            welcome_message: None,
        }
    }

    #[test]
    fn disallowed_user_is_ignored() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let mut config = open_config();
        config.allowed_users = vec!["U42".to_string()];
        let outcome = handle_command(&runtime, "C1", Some("U7"), "hi", &config).unwrap();
        assert_eq!(outcome, CommandOutcome::Ignored);
    }

    #[test]
    fn empty_message_is_ignored() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let outcome = handle_command(&runtime, "C1", Some("U1"), "   ", &open_config()).unwrap();
        assert_eq!(outcome, CommandOutcome::Ignored);
    }

    #[test]
    fn start_returns_welcome_without_touching_the_agent() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let mut config = open_config();
        config.welcome_message = Some("welcome!".to_string());
        let outcome = handle_command(&runtime, "C1", Some("U1"), "/start", &config).unwrap();
        assert_eq!(outcome, CommandOutcome::Reply("welcome!".to_string()));
    }

    #[test]
    fn help_command_is_handled_locally() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let outcome = handle_command(&runtime, "C1", Some("U1"), "/help", &open_config()).unwrap();
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
        let key = ConversationKey::new(PLATFORM_ID, "C555");
        // Seed the map so /new has something to remove.
        runtime.reset_conversation(&key).unwrap();
        // After /new the next session_for lookup is None (no dispatch
        // happened to create one).
        let outcome =
            handle_command(&runtime, "C555", Some("U1"), "/new", &open_config()).unwrap();
        assert!(matches!(outcome, CommandOutcome::Reply(_)));
        assert!(runtime.session_for(&key).unwrap().is_none());
    }

    #[test]
    fn allowed_user_passes_filter() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let mut config = open_config();
        config.allowed_users = vec!["U42".to_string()];
        // /help avoids needing a real provider.
        let outcome = handle_command(&runtime, "C1", Some("U42"), "/help", &config).unwrap();
        assert!(matches!(outcome, CommandOutcome::Reply(_)));
    }

    #[test]
    fn conversation_key_includes_thread_ts_when_present() {
        let bare = conversation_key("C1", None);
        assert_eq!(bare.conversation, "C1");

        let threaded = conversation_key("C1", Some("1700000000.000100"));
        assert_eq!(threaded.conversation, "C1:1700000000.000100");

        let empty_thread = conversation_key("C1", Some(""));
        assert_eq!(empty_thread.conversation, "C1");
    }
}
