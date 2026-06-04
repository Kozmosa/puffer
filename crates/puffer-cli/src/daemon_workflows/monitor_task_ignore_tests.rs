use super::{
    handle_monitor_task_ignore, monitor_ignore_filter, monitor_tasks_path,
    sync_monitor_ignore_filters_from_tasks,
};
use puffer_config::ConfigPaths;
use serde_json::{json, Value};
use std::fs;

#[test]
fn ignore_monitor_task_marks_task_and_appends_memory() {
    let tempdir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    let memory_path = tempdir.path().join("feed-connection.md");
    let task_path = monitor_tasks_path(&paths);
    fs::create_dir_all(task_path.parent().unwrap()).unwrap();
    fs::write(
        &task_path,
        serde_json::to_string_pretty(&json!({
            "tasks": [{
                "task_id": "monitor-1",
                "subject": "Noisy alert",
                "description": "Ignore this class of alert.",
                "status": "pending",
                "metadata": {
                    "_monitor": true,
                    "monitor_connection": "feed-connection",
                    "monitor_memory_path": memory_path.display().to_string(),
                    "possibleIgnoreReasons": ["low signal alert"]
                },
                "started_at_ms": 10
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let snapshot = handle_monitor_task_ignore(
        &paths,
        &json!({
            "taskId": "monitor-1",
            "reason": "low signal alert"
        }),
    )
    .unwrap();
    let tasks = snapshot["tasks"].as_array().unwrap();
    assert_eq!(tasks[0]["ignored"], Value::Bool(true));
    assert_eq!(tasks[0]["status"], "completed");
    assert_eq!(tasks[0]["task_id"], "monitor-1");

    let stored: Value = serde_json::from_str(&fs::read_to_string(task_path).unwrap()).unwrap();
    assert_eq!(
        stored["tasks"][0]["metadata"]["ignore_reason"],
        "low signal alert"
    );
    let memory = fs::read_to_string(memory_path).unwrap();
    assert!(memory.contains("Ignored Task: monitor-1"));
    assert!(memory.contains("Reason: low signal alert"));
    assert!(memory.contains("Title: Noisy alert"));
}

#[test]
fn ignore_conversational_task_dismisses_only_that_task() {
    // Regression for agentenv/monorepo#545: ignoring one Telegram task must
    // not sweep sibling tasks from the same chat/sender, and must not install
    // a source-muting ignore filter (which would also drop future messages).
    let tempdir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    let memory_path = tempdir.path().join("telegram-user.md");
    let task_path = monitor_tasks_path(&paths);
    fs::create_dir_all(task_path.parent().unwrap()).unwrap();
    let sibling = |task_id: &str, subject: &str| {
        json!({
            "task_id": task_id,
            "subject": subject,
            "description": "request from the same person",
            "status": "pending",
            "metadata": {
                "_monitor": true,
                "monitor_connection": "telegram-user",
                "monitor_connector": "telegram-login",
                "monitor_memory_path": memory_path.display().to_string(),
                "chat_id": 5229190700_i64,
                "sender_id": 674485095_i64
            }
        })
    };
    fs::write(
        &task_path,
        serde_json::to_string_pretty(&json!({
            "tasks": [
                sibling("monitor-1", "book a table"),
                sibling("monitor-2", "a different request")
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    handle_monitor_task_ignore(
        &paths,
        &json!({"taskId": "monitor-1", "reason": "not actionable"}),
    )
    .unwrap();

    let stored: Value = serde_json::from_str(&fs::read_to_string(&task_path).unwrap()).unwrap();
    // The ignored task is marked completed.
    assert_eq!(stored["tasks"][0]["status"], "completed");
    assert_eq!(stored["tasks"][0]["metadata"]["ignored"], true);
    // No source-muting filter is installed for a conversational connector.
    assert_eq!(stored["tasks"][0]["metadata"]["ignore_filter"], Value::Null);
    // The sibling from the same chat+sender stays pending.
    assert_eq!(stored["tasks"][1]["status"], "pending");
    assert_eq!(stored["tasks"][1]["metadata"]["ignored"], Value::Null);
}

#[test]
fn ignore_filter_derives_scoped_actor_identity() {
    let metadata = json!({
        "room_id": "security-room",
        "author_handle": "alert-bot"
    });
    let filter = monitor_ignore_filter(metadata.as_object().unwrap()).unwrap();

    assert_eq!(
        serde_json::to_value(filter).unwrap(),
        json!({
            "room_id": "security-room",
            "author_handle": "alert-bot"
        })
    );
}

#[test]
fn ignore_filter_accepts_explicit_safe_exact_filter() {
    let metadata = json!({
        "monitor_ignore_filter": {
            "conversation_id": "room-1",
            "from_email": "alerts@example.test"
        }
    });
    let filter = monitor_ignore_filter(metadata.as_object().unwrap()).unwrap();

    assert_eq!(
        serde_json::to_value(filter).unwrap(),
        json!({
            "conversation_id": "room-1",
            "from_email": "alerts@example.test"
        })
    );
}

#[test]
fn ignore_filter_rejects_single_or_unstable_identity() {
    let single = json!({
        "author_handle": "alert-bot"
    });
    assert!(monitor_ignore_filter(single.as_object().unwrap()).is_none());

    let unstable = json!({
        "room_id": "security-room",
        "sender_name": "Alert Bot"
    });
    assert!(monitor_ignore_filter(unstable.as_object().unwrap()).is_none());
}

#[test]
fn sync_marks_pending_tasks_matching_prior_ignore_filter() {
    let tempdir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    let task_path = monitor_tasks_path(&paths);
    fs::create_dir_all(task_path.parent().unwrap()).unwrap();
    fs::write(
        &task_path,
        serde_json::to_string_pretty(&json!({
            "tasks": [
                {
                    "task_id": "monitor-1",
                    "subject": "ignored bot",
                    "description": "old ignored example",
                    "status": "completed",
                    "metadata": {
                        "_monitor": true,
                        "monitor_connection": "feed-connection",
                        "monitor_connector": "gmail-browser",
                        "room_id": "security-room",
                        "author_handle": "alert-bot",
                        "ignored": true
                    }
                },
                {
                    "task_id": "monitor-2",
                    "subject": "same bot",
                    "description": "new pending example",
                    "status": "pending",
                    "metadata": {
                        "_monitor": true,
                        "monitor_connection": "feed-connection",
                        "monitor_connector": "gmail-browser",
                        "room_id": "security-room",
                        "author_handle": "alert-bot"
                    }
                },
                {
                    "task_id": "monitor-3",
                    "subject": "other sender",
                    "description": "should stay pending",
                    "status": "pending",
                    "metadata": {
                        "_monitor": true,
                        "monitor_connection": "feed-connection",
                        "monitor_connector": "gmail-browser",
                        "room_id": "security-room",
                        "author_handle": "human-user"
                    }
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    sync_monitor_ignore_filters_from_tasks(&paths).unwrap();

    let stored: Value = serde_json::from_str(&fs::read_to_string(task_path).unwrap()).unwrap();
    assert_eq!(stored["tasks"][1]["status"], "completed");
    assert_eq!(stored["tasks"][1]["metadata"]["ignored"], true);
    assert_eq!(
        stored["tasks"][1]["metadata"]["ignore_reason"],
        "Matched previous monitor ignore filter."
    );
    assert_eq!(stored["tasks"][2]["status"], "pending");
    assert_eq!(stored["tasks"][2]["metadata"]["ignored"], Value::Null);
}

#[test]
fn sync_does_not_parse_unstructured_task_text() {
    let tempdir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    let task_path = monitor_tasks_path(&paths);
    fs::create_dir_all(task_path.parent().unwrap()).unwrap();
    fs::write(
        &task_path,
        serde_json::to_string_pretty(&json!({
            "tasks": [
                {
                    "task_id": "monitor-1",
                    "subject": "ignored bot",
                    "description": "old ignored example",
                    "status": "completed",
                    "metadata": {
                        "_monitor": true,
                        "monitor_connection": "feed-connection",
                        "monitor_connector": "gmail-browser",
                        "room_id": "security-room",
                        "author_handle": "alert-bot",
                        "ignored": true
                    }
                },
                {
                    "task_id": "monitor-2",
                    "subject": "Review alert from alert bot",
                    "description": "Source author is alert-bot in security-room.",
                    "status": "pending",
                    "metadata": {
                        "_monitor": true,
                        "monitor_connection": "feed-connection",
                        "monitor_connector": "gmail-browser"
                    }
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    sync_monitor_ignore_filters_from_tasks(&paths).unwrap();

    let stored: Value = serde_json::from_str(&fs::read_to_string(task_path).unwrap()).unwrap();
    assert_eq!(stored["tasks"][1]["status"], "pending");
    assert_eq!(stored["tasks"][1]["metadata"]["ignored"], Value::Null);
}
