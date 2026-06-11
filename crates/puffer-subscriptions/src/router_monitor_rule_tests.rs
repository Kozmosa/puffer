#[cfg(test)]
mod monitor_rule_tests {
    use super::*;
    use crate::action::{ActionResult, BuiltinActionDispatcher};
    use crate::classify::NullClassifier;
    use crate::spec::{ActionSpec, TaggedFilterSpec, WorkflowBindingSpec};
    use puffer_subscriber_runtime::{Event, EventEnvelope};
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn keyword_ignore_filter_suppresses_matching_text() {
        let dir = tempdir().unwrap();
        let store = WorkflowBindingStore::load(dir.path().join("bindings.json")).unwrap();
        store
            .create(WorkflowBindingSpec {
                slug: "monitor-telegram-user".into(),
                description: "Monitor telegram-user for actionable tasks".into(),
                connection_slug: "telegram-user".into(),
                connector_slug: Some("telegram-login".into()),
                status: WorkflowBindingStatus::Enabled,
                filter: None,
                ignore_filters: vec![FilterSpec::Tagged(TaggedFilterSpec::Regex {
                    pattern: regex::escape("作业"),
                    case_insensitive: true,
                })],
                contact_ids: Vec::new(),
                classify_prompt: None,
                classify_model: None,
                action: ActionSpec::RunWorkflow {
                    slug: "downstream".into(),
                },
                created_at_ms: 0,
            })
            .unwrap();
        let dispatcher: Arc<dyn ActionDispatcher> = Arc::new(BuiltinActionDispatcher::new());
        let classifier: Arc<dyn Classifier> = Arc::new(NullClassifier);
        let history_store = WorkflowHistoryStore::load(dir.path().join("history.json")).unwrap();
        let matching = EventEnvelope {
            envelope_id: "env-matching-keyword".into(),
            subscriber_id: "telegram-user".into(),
            received_at_ms: 0,
            event: Event {
                topic: "telegram-user".into(),
                kind: "message".into(),
                control: false,
                dedup_key: None,
                text: "今天作业很多".into(),
                payload: serde_json::json!({"chat_id": 7_i64}),
            },
        };
        let non_matching = EventEnvelope {
            envelope_id: "env-non-matching-keyword".into(),
            event: Event {
                text: "今天正常消息".into(),
                ..matching.event.clone()
            },
            ..matching.clone()
        };

        let ignored = process_envelope_result(
            &matching,
            &store,
            Some(&history_store),
            &dispatcher,
            &classifier,
            None,
        );
        let passed =
            process_envelope_result(&non_matching, &store, None, &dispatcher, &classifier, None);

        assert!(!ignored.matched);
        assert_eq!(ignored.acted, 0);
        assert_eq!(
            history_store.list()[0].action_log[0].action,
            "monitor_ignore_filter"
        );
        assert!(passed.matched);
    }

    #[test]
    fn include_filter_skips_events_that_do_not_match_before_action() {
        struct OkDispatcher;

        impl ActionDispatcher for OkDispatcher {
            fn dispatch(&self, _action: &ActionSpec, _envelope: &EventEnvelope) -> ActionResult {
                ActionResult::success("ok")
            }
        }

        let dir = tempdir().unwrap();
        let store = WorkflowBindingStore::load(dir.path().join("bindings.json")).unwrap();
        store
            .create(WorkflowBindingSpec {
                slug: "monitor-telegram-user".into(),
                description: "Monitor telegram-user for actionable tasks".into(),
                connection_slug: "telegram-user".into(),
                connector_slug: Some("telegram-login".into()),
                status: WorkflowBindingStatus::Enabled,
                filter: Some(FilterSpec::Tagged(TaggedFilterSpec::Regex {
                    pattern: regex::escape("review"),
                    case_insensitive: true,
                })),
                ignore_filters: Vec::new(),
                contact_ids: Vec::new(),
                classify_prompt: None,
                classify_model: None,
                action: ActionSpec::RunWorkflow {
                    slug: "downstream".into(),
                },
                created_at_ms: 0,
            })
            .unwrap();
        let dispatcher: Arc<dyn ActionDispatcher> = Arc::new(OkDispatcher);
        let classifier: Arc<dyn Classifier> = Arc::new(NullClassifier);
        let base = EventEnvelope {
            envelope_id: "env-skip".into(),
            subscriber_id: "telegram-user".into(),
            received_at_ms: 0,
            event: Event {
                topic: "telegram-user".into(),
                kind: "message".into(),
                control: false,
                dedup_key: None,
                text: "hello".into(),
                payload: serde_json::json!({"chat_id": 7_i64}),
            },
        };
        let matching = EventEnvelope {
            envelope_id: "env-pass".into(),
            event: Event {
                text: "please review this".into(),
                ..base.event.clone()
            },
            ..base.clone()
        };

        let skipped = process_envelope_result(&base, &store, None, &dispatcher, &classifier, None);
        let passed =
            process_envelope_result(&matching, &store, None, &dispatcher, &classifier, None);

        assert!(!skipped.matched);
        assert_eq!(skipped.acted, 0);
        assert!(passed.matched);
        assert_eq!(passed.acted, 1);
    }
}
