use super::*;

#[test]
fn native_status_does_not_treat_lazy_initialization_as_error() {
    if CEF_INITIALIZATION.get().is_some() {
        return;
    }
    let status = browser_cef_native_status();
    assert_eq!(status["active"], serde_json::json!(false));
    assert_ne!(
        status["error"],
        serde_json::json!("native CEF was not initialized before the desktop event loop started")
    );
}

#[test]
fn native_state_before_initialization_is_disconnected() {
    if CEF_INITIALIZATION.get().is_some() {
        return;
    }
    let state = browser_cef_native_state("tab-1".to_string()).unwrap();
    assert_eq!(state["connected"], serde_json::json!(false));
    assert_eq!(state["url"], serde_json::json!("about:blank"));
}
