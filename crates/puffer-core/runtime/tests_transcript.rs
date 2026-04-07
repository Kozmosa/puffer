use super::*;
use super::tests::{plan_mode_state, state};
use serde_json::json;

#[test]
fn transcript_to_anthropic_messages_replays_all_roles() {
    let mut state = state();
    state.push_message(crate::MessageRole::User, "hello");
    state.push_message(crate::MessageRole::Assistant, "hi");
    state.push_message(crate::MessageRole::System, "note");

    let messages = transcript_to_anthropic_messages(&state, "fallback");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[2]["content"], "[system]\nnote");
}

#[test]
fn transcript_to_openai_input_replays_transcript_items() {
    let mut state = state();
    state.push_message(crate::MessageRole::User, "hello");
    state.push_message(crate::MessageRole::Assistant, "hi");

    let input = transcript_to_openai_input(&state, "fallback").unwrap();
    let items = input.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["role"], "user");
    assert_eq!(items[1]["type"], "message");
    assert_eq!(items[1]["role"], "assistant");
}

#[test]
fn transcript_to_openai_chat_messages_replays_transcript_items() {
    let mut state = state();
    state.push_message(crate::MessageRole::User, "hello");
    state.push_message(crate::MessageRole::Assistant, "hi");

    let messages = transcript_to_openai_chat_messages(&state, "fallback").unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[0].content, Some(json!("hello")));
}

#[test]
fn transcript_to_openai_chat_messages_preserves_system_role() {
    let mut state = state();
    state.push_message(crate::MessageRole::System, "rules");
    state.push_message(crate::MessageRole::User, "hello");
    state.push_message(crate::MessageRole::Assistant, "hi");

    let messages = transcript_to_openai_chat_messages(&state, "fallback").unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[2].role, "assistant");
}

#[test]
fn openai_input_includes_plan_mode_system_context() {
    let state = plan_mode_state();
    let input = transcript_to_openai_input(&state, "hello").unwrap();
    let items = input.as_array().unwrap();
    assert_eq!(items[0]["role"], "system");
    assert!(items[0]["content"]
        .as_str()
        .unwrap()
        .contains("Plan mode is active"));
    assert!(items[0]["content"]
        .as_str()
        .unwrap()
        .contains("Current plan contents:"));
}

#[test]
fn openai_chat_messages_include_plan_mode_system_context() {
    let state = plan_mode_state();
    let messages = transcript_to_openai_chat_messages(&state, "hello").unwrap();
    assert_eq!(messages[0].role, "system");
    assert!(messages[0]
        .content
        .as_ref()
        .unwrap()
        .as_str()
        .unwrap()
        .contains("Plan mode is active"));
}
