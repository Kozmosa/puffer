use super::super::handle_workflow_list;
use super::{handle_monitor_rule_add, handle_monitor_rule_delete};
use puffer_config::ConfigPaths;
use puffer_core::{install_subscription_manager, subscription_manager};
use puffer_subscriptions::{
    ActionSpec, FilterSpec, SubscriptionManager, SubscriptionManagerBuilder, TaggedFilterSpec,
    WorkflowBindingSpec, WorkflowBindingStatus,
};
use serde_json::json;
use std::fs;
use std::sync::{Arc, OnceLock};

struct TestManager {
    _runtime: tokio::runtime::Runtime,
    _tempdir: tempfile::TempDir,
    manager: Arc<SubscriptionManager>,
}

static TEST_MANAGER: OnceLock<TestManager> = OnceLock::new();

fn test_manager() -> Arc<SubscriptionManager> {
    if let Ok(manager) = subscription_manager() {
        return manager;
    }
    let state = TEST_MANAGER.get_or_init(|| {
        let tempdir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(1)
            .thread_name("puffer-monitor-rules-test")
            .build()
            .unwrap();
        let manager = Arc::new(
            SubscriptionManagerBuilder::new(tempdir.path().join("subscriptions.json"))
                .build(runtime.handle().clone())
                .unwrap(),
        );
        let _ = install_subscription_manager(manager.clone());
        TestManager {
            _runtime: runtime,
            _tempdir: tempdir,
            manager,
        }
    });
    subscription_manager().unwrap_or_else(|_| state.manager.clone())
}

fn monitor_binding(slug: &str, connection_slug: &str) -> WorkflowBindingSpec {
    WorkflowBindingSpec {
        slug: slug.to_string(),
        description: format!("Monitor {connection_slug} for actionable tasks"),
        connection_slug: connection_slug.to_string(),
        connector_slug: Some("telegram-login".to_string()),
        status: WorkflowBindingStatus::Enabled,
        filter: None,
        ignore_filters: Vec::new(),
        contact_ids: Vec::new(),
        classify_prompt: None,
        classify_model: None,
        action: ActionSpec::TriageAgent {
            prompt: "triage".to_string(),
            model: None,
        },
        created_at_ms: 0,
    }
}

#[test]
fn add_exclude_rule_persists_keyword_filter_and_ignores_legacy_scope() {
    let tempdir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    let manager = test_manager();
    let connection_slug = "rule-exclude-telegram";
    manager
        .store()
        .upsert(monitor_binding(
            &format!("monitor-{connection_slug}"),
            connection_slug,
        ))
        .unwrap();

    let snapshot = handle_monitor_rule_add(
        &paths,
        &json!({
            "connection_slug": connection_slug,
            "mode": "exclude",
            "keywords": ["作业"],
            "scope": {"field": "chat_id", "value": 7},
            "case_insensitive": true
        }),
    )
    .unwrap();

    let binding = manager
        .store()
        .get(&format!("monitor-{connection_slug}"))
        .unwrap();
    assert!(matches!(
        binding.ignore_filters.as_slice(),
        [FilterSpec::Tagged(TaggedFilterSpec::Regex { pattern, case_insensitive })]
            if pattern == "作业" && *case_insensitive
    ));
    assert_eq!(
        snapshot["workflow_bindings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|binding| binding["connection_slug"] == connection_slug)
            .unwrap()["ignore_filters"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn add_include_rule_persists_filter_without_touching_other_monitor() {
    let tempdir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    let manager = test_manager();
    let connection_slug = "rule-include-telegram";
    let other_connection_slug = "rule-include-other";
    manager
        .store()
        .upsert(monitor_binding(
            &format!("monitor-{connection_slug}"),
            connection_slug,
        ))
        .unwrap();
    manager
        .store()
        .upsert(monitor_binding(
            &format!("monitor-{other_connection_slug}"),
            other_connection_slug,
        ))
        .unwrap();

    handle_monitor_rule_add(
        &paths,
        &json!({
            "connection_slug": connection_slug,
            "mode": "include",
            "keywords": ["review"],
            "case_insensitive": true
        }),
    )
    .unwrap();

    let binding = manager
        .store()
        .get(&format!("monitor-{connection_slug}"))
        .unwrap();
    assert!(matches!(
        binding.filter,
        Some(FilterSpec::Tagged(TaggedFilterSpec::Regex { .. }))
    ));
    let other = manager
        .store()
        .get(&format!("monitor-{other_connection_slug}"))
        .unwrap();
    assert!(other.filter.is_none());
    assert!(other.ignore_filters.is_empty());
}

#[test]
fn delete_rule_removes_exact_displayed_rule() {
    let tempdir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    let manager = test_manager();
    let connection_slug = "rule-delete-telegram";
    manager
        .store()
        .upsert(monitor_binding(
            &format!("monitor-{connection_slug}"),
            connection_slug,
        ))
        .unwrap();
    let snapshot = handle_monitor_rule_add(
        &paths,
        &json!({
            "connection_slug": connection_slug,
            "mode": "exclude",
            "keywords": ["noise"],
            "case_insensitive": true
        }),
    )
    .unwrap();
    let rule = snapshot["workflow_bindings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|binding| binding["connection_slug"] == connection_slug)
        .unwrap()["ignore_filters"][0]
        .clone();

    handle_monitor_rule_delete(
        &paths,
        &json!({
            "connection_slug": connection_slug,
            "mode": "exclude",
            "rule": rule
        }),
    )
    .unwrap();

    let binding = manager
        .store()
        .get(&format!("monitor-{connection_slug}"))
        .unwrap();
    assert!(binding.ignore_filters.is_empty());
}

#[test]
fn workflow_snapshot_does_not_emit_scope_options() {
    let tempdir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    let manager = test_manager();
    let connection_slug = "rule-keyword-only-telegram";
    manager
        .store()
        .upsert(monitor_binding(
            &format!("monitor-{connection_slug}"),
            connection_slug,
        ))
        .unwrap();
    let task_path = paths
        .workspace_config_dir
        .join("runtime")
        .join("claude_workflow")
        .join("monitor_tasks.json");
    fs::create_dir_all(task_path.parent().unwrap()).unwrap();
    fs::write(
        &task_path,
        serde_json::to_string_pretty(&json!({
            "tasks": [{
                "task_id": "monitor-1",
                "subject": "Telegram group task",
                "description": "from group",
                "status": "pending",
                "metadata": {
                    "_monitor": true,
                    "monitor_connection": connection_slug,
                    "monitor_connector": "telegram-login",
                    "chat_id": 7,
                    "chat_title": "Puffer group",
                    "chat_kind": "group"
                }
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let snapshot = handle_workflow_list(&paths).unwrap();
    let binding = snapshot["workflow_bindings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|binding| binding["connection_slug"] == connection_slug)
        .unwrap();

    assert!(binding.get("scope_options").is_none());
}
