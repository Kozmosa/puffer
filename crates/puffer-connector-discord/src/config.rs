use serde::{Deserialize, Serialize};

/// Platform-specific configuration parsed from
/// `[connectors.discord]` in the Puffer config file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordConfig {
    /// Bot token issued by the Discord developer portal.
    pub token: String,

    /// Numeric Discord user ids permitted to talk to the bot. When
    /// empty or absent the bot accepts messages from everyone.
    #[serde(default)]
    pub allowed_users: Vec<u64>,

    /// Optional greeting sent in response to `/start`.
    #[serde(default)]
    pub welcome_message: Option<String>,
}

impl DiscordConfig {
    /// Returns `true` when `user_id` may talk to the bot.
    pub fn is_user_allowed(&self, user_id: u64) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.contains(&user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_allowed_list_means_public_bot() {
        let config = DiscordConfig {
            token: "t".to_string(),
            allowed_users: Vec::new(),
            welcome_message: None,
        };
        assert!(config.is_user_allowed(123));
        assert!(config.is_user_allowed(0));
    }

    #[test]
    fn allowed_list_filters_foreign_users() {
        let config = DiscordConfig {
            token: "t".to_string(),
            allowed_users: vec![1, 2, 3],
            welcome_message: None,
        };
        assert!(config.is_user_allowed(2));
        assert!(!config.is_user_allowed(99));
    }

    #[test]
    fn config_parses_from_raw_json() {
        let raw = serde_json::json!({
            "token": "abc",
            "allowed_users": [42, 7],
            "welcome_message": "hi"
        });
        let config: DiscordConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(config.token, "abc");
        assert_eq!(config.allowed_users, vec![42, 7]);
        assert_eq!(config.welcome_message.as_deref(), Some("hi"));
    }

    #[test]
    fn allowed_users_defaults_to_empty_when_missing() {
        let raw = serde_json::json!({ "token": "abc" });
        let config: DiscordConfig = serde_json::from_value(raw).unwrap();
        assert!(config.allowed_users.is_empty());
        assert!(config.welcome_message.is_none());
    }
}
