use super::*;

#[test]
fn try_open_overlay_builds_resume_picker() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let current = session_store
        .create_session(tempdir.path().join("current"))
        .unwrap();
    session_store
        .create_session(tempdir.path().join("dockyard"))
        .unwrap();

    let mut state = sample_state();
    state.session.id = current.id;
    let resources = sample_resources();
    let mut providers = sample_providers();
    let auth_store = sample_auth_store();
    let mut tui = TuiState::default();
    let opened = try_open_overlay(
        &state,
        &resources,
        &mut providers,
        &auth_store,
        &session_store,
        &mut tui,
        "/resume",
    )
    .unwrap();
    assert!(opened);
    match tui.overlay {
        Some(OverlayState::SessionPicker { sessions, .. }) => {
            assert_eq!(sessions.len(), 1);
            assert_ne!(sessions[0].id, current.id);
        }
        _ => panic!("resume picker"),
    }
}

#[test]
fn try_open_overlay_builds_agent_picker() {
    let tempdir = tempdir().unwrap();
    std::fs::create_dir_all(tempdir.path().join("resources/agents")).unwrap();
    std::fs::write(
        tempdir.path().join("resources/agents/reviewer.yaml"),
        "id: reviewer\ndescription: Reviews code carefully.\nprompt: |\n  You are a reviewer.\ntools:\n  - Read\nmodel: openai/gpt-5\n",
    )
    .unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();

    let mut state = sample_state();
    state.cwd = tempdir.path().to_path_buf();
    let resources = sample_resources();
    let mut providers = sample_providers();
    let auth_store = sample_auth_store();
    let mut tui = TuiState::default();
    let opened = try_open_overlay(
        &state,
        &resources,
        &mut providers,
        &auth_store,
        &session_store,
        &mut tui,
        "/agents",
    )
    .unwrap();
    assert!(opened);
    let entries = match &tui.overlay {
        Some(OverlayState::AgentPicker { entries, .. }) => entries,
        _ => panic!("agent picker"),
    };
    assert!(entries.iter().any(|entry| entry.selector == "reviewer"));
}
