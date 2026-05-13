use super::*;

#[test]
fn browser_and_bridge_session_grants_feed_profile_categories() {
    let mut grants = SessionPermissionGrants::default();
    let session_id = uuid::Uuid::parse_str("2ba8b01d-5e7a-46b6-b747-7bfe5f6fa36a").unwrap();
    grants.grant_tool_call(
        &tool_definition("Browser", "on-request"),
        &serde_json::json!({
            "action": "evaluate",
            "sessionId": "b4f239fd-1493-4be7-a3a1-9e58fe612576",
            "script": "1+1"
        }),
        &session_id,
    );
    grants.grant_tool_call(
        &tool_definition("SendMessage", "auto"),
        &serde_json::json!({"to":"bridge:session-123","message":"hi"}),
        &session_id,
    );
    let profile = EffectivePermissionProfile::from_session_state(
        PathBuf::from("/repo").as_path(),
        &[],
        &PermissionsSettings::default(),
        &SandboxSettings::from_mode("workspace-write"),
        &session_id,
        &SessionPermissionState::new(false, grants),
        false,
        None,
        None,
    );

    assert!(profile
        .grants
        .surface_grants
        .contains(&PermissionSurface::Browser));
    assert!(profile
        .grants
        .surface_grants
        .contains(&PermissionSurface::Workflow));
    assert!(!profile.grants.tool_overrides.contains_key("browser"));
    assert_eq!(
        profile
            .grants
            .tool_overrides
            .get("send_message")
            .copied()
            .or_else(|| profile.grants.tool_overrides.get("sendmessage").copied()),
        Some(EffectiveApprovalPolicy::Allow)
    );
    assert!(profile.grants.category_grants.iter().any(|grant| matches!(
        grant,
        PermissionGrantCategory::Browser(BrowserGrantCategory::Action(
            BrowserActionCategory::Evaluate
        ))
    )));
    assert!(profile.grants.category_grants.iter().any(|grant| matches!(
        grant,
        PermissionGrantCategory::Browser(BrowserGrantCategory::CrossSessionAccess)
    )));
    assert!(profile.grants.category_grants.iter().any(|grant| matches!(
        grant,
        PermissionGrantCategory::Workflow(
            crate::permissions::profile::WorkflowGrantCategory::CrossSessionBridge
        )
    )));
}

#[test]
fn coarse_workspace_browser_override_is_still_supported_as_input() {
    let context = runtime_context(
        PermissionsSettings {
            tools: BTreeMap::from([("browser".to_string(), "allow".to_string())]),
        },
        SandboxSettings::from_mode("workspace-write"),
        false,
        None,
        PathBuf::from("/tmp"),
        Vec::new(),
        SessionPermissionState::default(),
    );

    let decision = context.decision_for_tool_call(
        &tool_definition("Browser", "auto"),
        &serde_json::json!({
            "action":"evaluate",
            "script":"document.title"
        }),
    );

    assert_eq!(decision.behavior, ToolPermissionBehavior::Allow);
}

#[test]
fn browser_scope_defaults_to_current_session_and_inspect() {
    let context = runtime_context(
        PermissionsSettings::default(),
        SandboxSettings::from_mode("workspace-write"),
        false,
        None,
        PathBuf::from("/tmp"),
        Vec::new(),
        SessionPermissionState::default(),
    );

    let scope = context
        .effective_profile()
        .browser_scope(&serde_json::json!({"action":"snapshot"}));

    assert_eq!(scope.action, Some(BrowserActionCategory::Inspect));
    assert_eq!(
        scope.root_session_id,
        "2ba8b01d-5e7a-46b6-b747-7bfe5f6fa36a".to_string()
    );
    assert!(!scope.is_cross_session);
}

#[test]
fn browser_inspect_is_allowed_but_navigation_and_evaluate_require_approval() {
    let context = runtime_context(
        PermissionsSettings::default(),
        SandboxSettings::from_mode("workspace-write"),
        false,
        None,
        PathBuf::from("/tmp"),
        Vec::new(),
        SessionPermissionState::default(),
    );

    let inspect = context.decision_for_tool_call(
        &tool_definition("Browser", "auto"),
        &serde_json::json!({
            "action":"snapshot"
        }),
    );
    let navigate = context.decision_for_tool_call(
        &tool_definition("Browser", "auto"),
        &serde_json::json!({
            "action":"navigate",
            "url":"https://example.com"
        }),
    );
    let evaluate = context.decision_for_tool_call(
        &tool_definition("Browser", "auto"),
        &serde_json::json!({
            "action":"evaluate",
            "script":"document.title"
        }),
    );

    assert_eq!(inspect.behavior, ToolPermissionBehavior::Allow);
    assert_eq!(navigate.behavior, ToolPermissionBehavior::Ask);
    assert_eq!(evaluate.behavior, ToolPermissionBehavior::Ask);
    assert!(evaluate
        .reason
        .unwrap_or_default()
        .contains("executes page JavaScript"));
}

#[test]
fn browser_cross_session_access_requires_explicit_approval() {
    let context = runtime_context(
        PermissionsSettings::default(),
        SandboxSettings::from_mode("workspace-write"),
        false,
        None,
        PathBuf::from("/tmp"),
        Vec::new(),
        SessionPermissionState::default(),
    );

    let decision = context.decision_for_tool_call(
        &tool_definition("Browser", "auto"),
        &serde_json::json!({
            "action":"snapshot",
            "sessionId":"b4f239fd-1493-4be7-a3a1-9e58fe612576"
        }),
    );

    assert_eq!(decision.behavior, ToolPermissionBehavior::Ask);
    assert!(decision
        .reason
        .unwrap_or_default()
        .contains("cross-session browser access"));
}

#[test]
fn browser_session_grant_is_scoped_to_action_category_and_cross_session_flag() {
    let session_id = Uuid::parse_str("2ba8b01d-5e7a-46b6-b747-7bfe5f6fa36a").unwrap();
    let mut grants = SessionPermissionGrants::default();
    grants.grant_tool_call(
        &tool_definition("Browser", "auto"),
        &serde_json::json!({
            "action":"evaluate",
            "script":"document.title"
        }),
        &session_id,
    );
    let profile = EffectivePermissionProfile::from_session_state(
        PathBuf::from("/repo").as_path(),
        &[],
        &PermissionsSettings::default(),
        &SandboxSettings::from_mode("workspace-write"),
        &session_id,
        &SessionPermissionState::new(false, grants),
        false,
        None,
        None,
    );

    assert!(profile.browser_session_grant_allows(&serde_json::json!({
        "action":"evaluate",
        "script":"window.location.href"
    })));
    assert!(!profile.browser_session_grant_allows(&serde_json::json!({
        "action":"snapshot"
    })));
    assert!(!profile.browser_session_grant_allows(&serde_json::json!({
        "action":"evaluate",
        "sessionId":"b4f239fd-1493-4be7-a3a1-9e58fe612576",
        "script":"window.location.href"
    })));
}

#[test]
fn browser_surface_only_session_grant_does_not_allow_evaluate() {
    let mut grants = SessionPermissionGrants::default();
    grants.grant_surface_for_test(PermissionSurface::Browser);

    let context = runtime_context_with_session_grants(
        PermissionsSettings::default(),
        SandboxSettings::from_mode("workspace-write"),
        false,
        None,
        PathBuf::from("/tmp"),
        Vec::new(),
        false,
        grants,
        RuntimePermissionInputs::default(),
    );

    let decision = context.decision_for_tool_call(
        &tool_definition("Browser", "auto"),
        &serde_json::json!({
            "action":"evaluate",
            "script":"document.title"
        }),
    );

    assert_eq!(decision.behavior, ToolPermissionBehavior::Ask);
}
