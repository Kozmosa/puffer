use super::{
    connection_slug_from_input, format_succinct_message_list, format_succinct_message_search,
    telegram_connection_description, telegram_connection_manifest,
};
use puffer_config::ConfigPaths;
use serde_json::json;

#[test]
fn connection_slug_defaults_and_validates() {
    assert_eq!(
        connection_slug_from_input(&json!({})).unwrap(),
        "telegram-user"
    );
    assert_eq!(
        connection_slug_from_input(&json!({"connection_slug": "tg-alt"})).unwrap(),
        "tg-alt"
    );
    assert_eq!(
        connection_slug_from_input(&json!({"account": "tg-alt"})).unwrap(),
        "tg-alt"
    );
    assert_eq!(
        connection_slug_from_input(&json!({"connection_slug": "telegram-login"})).unwrap(),
        "telegram-user"
    );
    assert!(connection_slug_from_input(&json!({"connection_slug": "TG Alt"})).is_err());
}

#[test]
fn telegram_connection_description_uses_login_payload() {
    let payload = json!({
        "user_id": 12345,
        "first_name": "Tony"
    });

    assert_eq!(
        telegram_connection_description(&payload),
        "Telegram Tony (12345)"
    );
}

#[test]
fn missing_manifest_error_lists_telegram_search_paths() {
    let root = tempfile::tempdir().unwrap();
    let paths = ConfigPaths {
        workspace_root: root.path().join("workspace"),
        workspace_config_dir: root.path().join("workspace").join(".puffer"),
        user_config_dir: root.path().join("home").join(".puffer"),
        builtin_resources_dir: root.path().join("bundle").join("resources"),
    };

    let error = telegram_connection_manifest(&paths, "telegram-user")
        .unwrap_err()
        .to_string();

    assert!(error.contains("telegram-user subscriber manifest not found"));
    assert!(error.contains(
        &paths
            .workspace_config_dir
            .join("subscribers/telegram-user/manifest.toml")
            .display()
            .to_string()
    ));
    assert!(error.contains(
        &paths
            .builtin_resources_dir
            .join("subscribers/telegram-user/manifest.toml")
            .display()
            .to_string()
    ));
}

#[test]
fn default_telegram_connection_uses_account_state_dir() {
    let root = tempfile::tempdir().unwrap();
    let paths = ConfigPaths {
        workspace_root: root.path().join("workspace"),
        workspace_config_dir: root.path().join("workspace").join(".puffer"),
        user_config_dir: root.path().join("home").join(".puffer"),
        builtin_resources_dir: root.path().join("bundle").join("resources"),
    };
    write_telegram_manifest(&paths);

    let manifest = telegram_connection_manifest(&paths, "telegram-user").unwrap();

    assert_eq!(manifest.spec.id, "telegram-user");
    assert_eq!(manifest.topic(), "telegram-user");
    assert_eq!(
        manifest.spec.state.unwrap().dir,
        paths
            .user_config_dir
            .join("telegram-accounts/telegram-user")
            .to_string_lossy()
    );
}

#[test]
fn alternate_telegram_connection_uses_account_state_dir() {
    let root = tempfile::tempdir().unwrap();
    let paths = ConfigPaths {
        workspace_root: root.path().join("workspace"),
        workspace_config_dir: root.path().join("workspace").join(".puffer"),
        user_config_dir: root.path().join("home").join(".puffer"),
        builtin_resources_dir: root.path().join("bundle").join("resources"),
    };
    write_telegram_manifest(&paths);

    let manifest = telegram_connection_manifest(&paths, "tg-alt").unwrap();

    assert_eq!(manifest.spec.id, "tg-alt");
    assert_eq!(manifest.topic(), "tg-alt");
    assert_eq!(
        manifest.spec.state.unwrap().dir,
        paths
            .user_config_dir
            .join("telegram-accounts/tg-alt")
            .to_string_lossy()
    );
}

fn write_telegram_manifest(paths: &ConfigPaths) {
    let manifest_dir = paths
        .builtin_resources_dir
        .join("subscribers")
        .join("telegram-user");
    std::fs::create_dir_all(&manifest_dir).unwrap();
    std::fs::write(
        manifest_dir.join("manifest.toml"),
        r#"manifest_version = 1
id = "telegram-user"
kind = "subscriber"
topic = "telegram-user"
display_name = "Telegram (user account)"

[run]
cmd = ["puffer", "__subscriber", "telegram-user"]

[state]
dir = "state"
"#,
    )
    .unwrap();
}

#[test]
fn succinct_message_search_formats_plain_text() {
    let payload = json!({
        "query": "karen",
        "count": 1,
        "limit_reached": true,
        "chat": {
            "id": "477843728",
            "kind": "user",
            "title": "Tony",
            "handle": "@tony"
        },
        "results": [{
            "match": {
                "id": 3,
                "offset": null,
                "from": "Tony",
                "outgoing": false,
                "is_match": true,
                "text": "karen"
            },
            "context": [
                {
                    "id": 1,
                    "offset": -1,
                    "from": "Me",
                    "outgoing": true,
                    "is_match": false,
                    "text": "before"
                },
                {
                    "id": 3,
                    "offset": 0,
                    "from": "Tony",
                    "outgoing": false,
                    "is_match": true,
                    "text": "karen"
                },
                {
                    "id": 4,
                    "offset": 1,
                    "from": "Me",
                    "outgoing": true,
                    "is_match": false,
                    "text": "after"
                }
            ],
            "context_error": null
        }]
    });

    let formatted = format_succinct_message_search(&payload);

    assert_eq!(
        formatted,
        concat!(
            "Telegram search: \"karen\" in Tony (@tony)\n",
            "1 match returned (limit reached)\n",
            "\n",
            "Match:\n",
            " -1 Me: before\n",
            "  0 Tony: karen\n",
            " +1 Me: after"
        )
    );
    assert!(!formatted.contains('{'));
    assert!(!formatted.contains("\"context\""));
}

#[test]
fn succinct_message_list_formats_sender_scan_metadata() {
    let payload = json!({
        "count": 1,
        "limit_reached": false,
        "sender_filter": "Tony",
        "scanned": 200,
        "scan_limit": 200,
        "scan_limit_reached": true,
        "next_before_id": 77,
        "chat": {
            "id": "477843728",
            "kind": "group",
            "title": "Ops"
        },
        "messages": [{
            "id": 81,
            "date": "2026-05-26T09:00:00Z",
            "from": "Tony",
            "outgoing": false,
            "text": "hi"
        }]
    });

    let formatted = format_succinct_message_list(&payload);

    assert_eq!(
        formatted,
        concat!(
            "Telegram messages in Ops\n",
            "Sender filter: Tony\n",
            "1 message returned\n",
            "Scanned 200/200 messages (scan limit reached)\n",
            "Older page cursor: --before-id 77\n",
            "#81 2026-05-26T09:00:00Z Tony: hi"
        )
    );
}
