use crate::handler::PLATFORM_ID;
use crate::SlackConfig;
use anyhow::anyhow;
use puffer_connector_core::{
    Connector, ConnectorHandle, ConnectorRuntime, ConnectorStartError,
};
use std::sync::Arc;

/// Slack connector ready to be started by the puffer connector hub.
///
/// # Live driver status
///
/// TODO(live-driver): The real Socket Mode listener is not wired up yet.
/// `start` currently returns a `ConnectorStartError::Other` so the crate
/// compiles and the handler logic is fully exercised by unit tests, but
/// no inbound Slack traffic is processed at runtime.
///
/// The intended shape, once slack-morphism 2.x is pulled in, is:
///
/// 1. Build a `SlackClient` with a `SlackClientHyperConnector`.
/// 2. Open a `SlackSocketModeListener` authenticated with `app_token`.
/// 3. On each inbound `message` event:
///    * extract `channel_id`, `user_id`, `thread_ts`, `text`;
///    * call `spawn_blocking(move || handle_command_threaded(...))`;
///    * send the resulting reply back with `chat.postMessage` on the
///      Web API using `bot_token`.
/// 4. On `oneshot::Receiver<()>` firing, call `shutdown` on the
///    listener's environment and exit the Tokio runtime.
///
/// The spawn-a-thread-with-its-own-Tokio-runtime scaffold mirrors the
/// Telegram connector exactly.
pub struct SlackConnector {
    config: SlackConfig,
}

impl SlackConnector {
    pub fn new(config: SlackConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &SlackConfig {
        &self.config
    }
}

impl Connector for SlackConnector {
    fn id(&self) -> &str {
        PLATFORM_ID
    }

    fn start(
        self: Box<Self>,
        _runtime: Arc<ConnectorRuntime>,
    ) -> Result<ConnectorHandle, ConnectorStartError> {
        if self.config.bot_token.trim().is_empty() {
            return Err(ConnectorStartError::MissingConfig {
                id: PLATFORM_ID.to_string(),
                detail: "bot_token is empty".to_string(),
            });
        }
        if self.config.app_token.trim().is_empty() {
            return Err(ConnectorStartError::MissingConfig {
                id: PLATFORM_ID.to_string(),
                detail: "app_token is empty".to_string(),
            });
        }

        // TODO(live-driver): replace this stub with a real slack-morphism
        // Socket Mode listener. Until then the connector refuses to start
        // rather than silently swallowing inbound Slack traffic.
        Err(ConnectorStartError::other(
            PLATFORM_ID,
            anyhow!(
                "slack connector live driver is not implemented yet; \
                 handler logic is available via puffer_connector_slack::handle_command"
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> SlackConfig {
        SlackConfig {
            bot_token: "xoxb-t".to_string(),
            app_token: "xapp-t".to_string(),
            allowed_users: Vec::new(),
            welcome_message: None,
        }
    }

    #[test]
    fn connector_reports_platform_id() {
        let connector = SlackConnector::new(sample_config());
        assert_eq!(connector.id(), PLATFORM_ID);
    }

    #[test]
    fn missing_bot_token_is_reported() {
        let mut config = sample_config();
        config.bot_token = "".to_string();
        let connector = Box::new(SlackConnector::new(config));
        let runtime = dummy_runtime();
        match connector.start(runtime) {
            Err(ConnectorStartError::MissingConfig { detail, .. }) => {
                assert!(detail.contains("bot_token"));
            }
            Err(other) => panic!("expected MissingConfig, got {other:?}"),
            Ok(_) => panic!("expected start to fail without bot_token"),
        }
    }

    fn dummy_runtime() -> Arc<ConnectorRuntime> {
        use puffer_config::{ConfigPaths, PufferConfig};
        use puffer_connector_core::{ConnectorRuntimeConfig, ConversationSessionMap};
        use puffer_provider_registry::{AuthStore, ProviderRegistry};
        use puffer_resources::LoadedResources;
        use puffer_session_store::SessionStore;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
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
}
