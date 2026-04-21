use serde::{Deserialize, Serialize};

/// Platform-specific configuration parsed from
/// `[connectors.slack]` in the Puffer config file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlackConfig {
    /// Bot User OAuth token (`xoxb-…`) used for Web API calls such as
    /// `chat.postMessage`.
    pub bot_token: String,

    /// App-level token (`xapp-…`) with `connections:write` used to open a
    /// Socket Mode connection. Required for the polling-friendly live
    /// driver; ignored by the handler itself.
    pub app_token: String,

    /// Slack user ids (e.g. `"U12345"`) permitted to talk to the bot. When
    /// empty or absent the bot accepts messages from everyone.
    #[serde(default)]
    pub allowed_users: Vec<String>,

    /// Optional greeting sent in response to `/start`.
    #[serde(default)]
    pub welcome_message: Option<String>,
}

impl SlackConfig {
    /// Returns `true` when `user_id` may talk to the bot.
    pub fn is_user_allowed(&self, user_id: &str) -> bool {
        self.allowed_users.is_empty()
            || self.allowed_users.iter().any(|u| u == user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_allowed_list_means_public_bot() {
        let config = SlackConfig {
            bot_token: "xoxb-1".to_string(),
            app_token: "xapp-1".to_string(),
            allowed_users: Vec::new(),
            welcome_message: None,
        };
        assert!(config.is_user_allowed("U123"));
        assert!(config.is_user_allowed("UANY"));
    }

    #[test]
    fn allowed_list_filters_foreign_users() {
        let config = SlackConfig {
            bot_token: "xoxb-1".to_string(),
            app_token: "xapp-1".to_string(),
            allowed_users: vec!["U1".to_string(), "U2".to_string(), "U3".to_string()],
            welcome_message: None,
        };
        assert!(config.is_user_allowed("U2"));
        assert!(!config.is_user_allowed("U99"));
    }

    #[test]
    fn config_parses_from_raw_json() {
        let raw = serde_json::json!({
            "bot_token": "xoxb-abc",
            "app_token": "xapp-abc",
            "allowed_users": ["U42", "U7"],
            "welcome_message": "hi"
        });
        let config: SlackConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(config.bot_token, "xoxb-abc");
        assert_eq!(config.app_token, "xapp-abc");
        assert_eq!(
            config.allowed_users,
            vec!["U42".to_string(), "U7".to_string()]
        );
        assert_eq!(config.welcome_message.as_deref(), Some("hi"));
    }

    #[test]
    fn allowed_users_defaults_to_empty_when_missing() {
        let raw = serde_json::json!({
            "bot_token": "xoxb-abc",
            "app_token": "xapp-abc"
        });
        let config: SlackConfig = serde_json::from_value(raw).unwrap();
        assert!(config.allowed_users.is_empty());
        assert!(config.welcome_message.is_none());
    }
}
