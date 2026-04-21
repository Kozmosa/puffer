use puffer_connector_core::{ConnectorRuntime, ConversationKey};
use std::sync::Arc;

/// Stable platform id stored in the conversation map.
pub(crate) const PLATFORM_ID: &str = "email";

/// Outcome of handling one inbound email. The connector uses this to
/// decide what to send back on the wire.
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
    /// The inbound sender is not permitted to talk to the bot. Silently
    /// ignore.
    Ignored,
}

/// Dispatches one inbound email. Pure logic — no IMAP or SMTP calls here
/// — so the full decision tree is unit-testable against an in-memory
/// runtime.
///
/// `thread_id` is the opaque conversation key for this email thread.
/// Callers derive it from the first `Message-ID` in the reply chain so
/// that follow-up replies land on the same Puffer session.
///
/// `sender` is the bare email address from the `From:` header. `subject`
/// and `body` are the message's parsed subject line and plain-text body.
/// A leading command marker like `/new` or `/help` may appear either as
/// the subject or as the first non-empty line of the body.
pub fn handle_command(
    runtime: &Arc<ConnectorRuntime>,
    thread_id: &str,
    sender: &str,
    subject: &str,
    body: &str,
    config: &crate::EmailConfig,
) -> anyhow::Result<CommandOutcome> {
    if !config.is_sender_allowed(sender) {
        return Ok(CommandOutcome::Ignored);
    }

    let body_trimmed = body.trim();
    let subject_trimmed = subject.trim();

    if body_trimmed.is_empty() && subject_trimmed.is_empty() {
        return Ok(CommandOutcome::Ignored);
    }

    let key = ConversationKey::new(PLATFORM_ID, thread_id.to_string());

    // Allow slash commands in either the subject line or the first line
    // of the body. Body takes precedence when both are present so users
    // who keep a running subject like "Re: Puffer chat" aren't surprised
    // by `/new` never firing.
    let command_source = if let Some(cmd) = first_line_command(body_trimmed) {
        Some(cmd)
    } else {
        first_line_command(subject_trimmed)
    };

    if let Some(command) = command_source {
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
                // Unknown commands fall through to the agent, just like
                // the Telegram connector.
            }
        }
    }

    // Prefer the body text for the agent; fall back to the subject when
    // the body is blank (common for one-liner subject-only emails).
    let input = if body_trimmed.is_empty() {
        subject_trimmed
    } else {
        body_trimmed
    };

    if input.is_empty() {
        return Ok(CommandOutcome::Ignored);
    }

    let outcome = runtime.dispatch(&key, input)?;
    Ok(CommandOutcome::AgentReply {
        session_id: outcome.session_id,
        created: outcome.created,
        text: outcome.assistant_text,
    })
}

/// Returns the slash-command suffix of the first non-empty line of
/// `text`, if that line starts with `/`.
fn first_line_command(text: &str) -> Option<&str> {
    let first_line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    first_line.strip_prefix('/')
}

fn default_welcome() -> String {
    "Puffer is online. Reply with any text to talk to the agent, or \
     send /help for commands."
        .to_string()
}

fn help_text() -> String {
    "/start — greeting\n\
     /new   — start a fresh session for this email thread\n\
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

    fn open_config() -> crate::EmailConfig {
        crate::EmailConfig {
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            username: "bot@example.com".to_string(),
            password: "secret".to_string(),
            from_address: "bot@example.com".to_string(),
            allowed_senders: Vec::new(),
            welcome_message: None,
            poll_interval_secs: None,
        }
    }

    #[test]
    fn disallowed_sender_is_ignored() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let mut config = open_config();
        config.allowed_senders = vec!["allowed@example.com".to_string()];
        let outcome =
            handle_command(&runtime, "thread-1", "stranger@example.com", "hi", "body", &config)
                .unwrap();
        assert_eq!(outcome, CommandOutcome::Ignored);
    }

    #[test]
    fn empty_body_and_subject_is_ignored() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let outcome = handle_command(
            &runtime,
            "thread-1",
            "user@example.com",
            "   ",
            "   ",
            &open_config(),
        )
        .unwrap();
        assert_eq!(outcome, CommandOutcome::Ignored);
    }

    #[test]
    fn start_returns_welcome_without_touching_the_agent() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let mut config = open_config();
        config.welcome_message = Some("welcome!".to_string());
        let outcome = handle_command(
            &runtime,
            "thread-1",
            "user@example.com",
            "anything",
            "/start",
            &config,
        )
        .unwrap();
        assert_eq!(outcome, CommandOutcome::Reply("welcome!".to_string()));
    }

    #[test]
    fn help_command_is_handled_locally() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let outcome = handle_command(
            &runtime,
            "thread-1",
            "user@example.com",
            "/help",
            "",
            &open_config(),
        )
        .unwrap();
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
        let thread_id = "msg-id-555";
        let key = ConversationKey::new(PLATFORM_ID, thread_id);
        // Seed the map so /new has something to remove.
        runtime.reset_conversation(&key).unwrap();
        let outcome = handle_command(
            &runtime,
            thread_id,
            "user@example.com",
            "whatever",
            "/new",
            &open_config(),
        )
        .unwrap();
        assert!(matches!(outcome, CommandOutcome::Reply(_)));
        assert!(runtime.session_for(&key).unwrap().is_none());
    }

    #[test]
    fn allowed_sender_passes_filter() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let mut config = open_config();
        config.allowed_senders = vec!["Alice@Example.com".to_string()];
        // /help avoids needing a real provider.
        let outcome = handle_command(
            &runtime,
            "thread-1",
            "alice@example.com",
            "/help",
            "",
            &config,
        )
        .unwrap();
        assert!(matches!(outcome, CommandOutcome::Reply(_)));
    }
}
