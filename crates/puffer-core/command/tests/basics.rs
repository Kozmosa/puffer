use super::*;
use puffer_config::{MascotConfig, UiConfig};
use puffer_session_store::SessionMetadata;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[test]
fn command_registry_contains_review_usage_and_resume_alias() {
    let commands = supported_commands();
    assert!(find_command(&commands, "review").is_some());
    assert!(find_command(&commands, "tag").is_some());
    assert!(find_command(&commands, "usage").is_some());
    assert!(find_command(&commands, "continue").is_some());
}

#[test]
fn app_state_defaults_expose_command_state() {
    let state = AppState::new(
        PufferConfig {
            app_name: "Puffer".to_string(),
            default_model: None,
            default_provider: Some("anthropic".to_string()),
            openai_base_url: None,
            openai_headers: BTreeMap::new(),
            openai_query_params: BTreeMap::new(),
            theme: "puffer".to_string(),
            mascot: MascotConfig {
                id: "clawd".to_string(),
                display_name: "Clawd".to_string(),
                enabled: true,
            },
            ui: UiConfig {
                no_alt_screen: false,
                tmux_golden_mode: false,
                status_line: None,
            },
        },
        PathBuf::from("."),
        SessionMetadata {
            id: uuid::Uuid::nil(),
            display_name: None,
            cwd: PathBuf::from("."),
            created_at_ms: 0,
            updated_at_ms: 0,
            parent_session_id: None,
            slug: None,
            tags: Vec::new(),
            note: None,
        },
    );
    assert_eq!(state.prompt_color, "default");
    assert_eq!(state.effort_level, "medium");
    assert_eq!(state.sandbox_mode, "workspace-write");
    assert!(state.statusline_enabled);
}

#[test]
fn local_commands_append_state_snapshots() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store
        .create_session(tempdir.path().to_path_buf())
        .unwrap();
    let mut state = AppState::new(
        PufferConfig::default(),
        tempdir.path().to_path_buf(),
        session.clone(),
    );

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut ProviderRegistry::new(),
        &mut AuthStore::default(),
        &session_store,
        "/theme harbor",
    )
    .unwrap();

    let record = session_store.load_session(session.id).unwrap();
    assert!(matches!(
        record.events.iter().rev().nth(1),
        Some(TranscriptEvent::StateSnapshot { theme, .. }) if theme == "harbor"
    ));
}

#[test]
fn resume_switches_to_matching_session_record() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let primary = session_store
        .create_session(tempdir.path().join("primary"))
        .unwrap();
    let secondary = session_store
        .create_session(tempdir.path().join("secondary"))
        .unwrap();
    session_store
        .rename_session(secondary.id, "dockyard".to_string())
        .unwrap();
    session_store
        .append_event(
            secondary.id,
            TranscriptEvent::StateSnapshot {
                current_model: Some("anthropic/claude-sonnet-4-5".to_string()),
                current_provider: Some("anthropic".to_string()),
                theme: "lagoon".to_string(),
                prompt_color: "teal".to_string(),
                effort_level: "high".to_string(),
                fast_mode: true,
                plan_mode: false,
                sandbox_mode: "workspace-write".to_string(),
                remote_name: None,
                remote_environment: None,
                remote_session_id: None,
                remote_session_url: None,
                remote_session_status: None,
                statusline_enabled: true,
                working_dirs: vec![tempdir.path().join("secondary").display().to_string()],
                claude_read_state: Vec::new(),
            },
        )
        .unwrap();

    let mut state = AppState::new(
        PufferConfig::default(),
        tempdir.path().join("primary"),
        primary,
    );
    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut ProviderRegistry::new(),
        &mut AuthStore::default(),
        &session_store,
        "/resume dockyard",
    )
    .unwrap();

    assert_eq!(state.session.id, secondary.id);
    assert_eq!(state.config.theme, "lagoon");
    assert_eq!(
        state.current_model.as_deref(),
        Some("anthropic/claude-sonnet-4-5")
    );
}

#[test]
fn resume_matches_session_slug() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let primary = session_store
        .create_session(tempdir.path().join("primary"))
        .unwrap();
    let secondary = session_store
        .create_session(tempdir.path().join("secondary"))
        .unwrap();
    session_store
        .set_slug(secondary.id, Some("dockyard-run".to_string()))
        .unwrap();

    let mut state = AppState::new(
        PufferConfig::default(),
        tempdir.path().join("primary"),
        primary,
    );
    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut ProviderRegistry::new(),
        &mut AuthStore::default(),
        &session_store,
        "/resume dockyard-run",
    )
    .unwrap();

    assert_eq!(state.session.id, secondary.id);
}

#[test]
fn resume_reports_ambiguous_matches_without_switching_sessions() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let primary = session_store
        .create_session(tempdir.path().join("primary"))
        .unwrap();
    let first = session_store
        .create_session(tempdir.path().join("dockyard-a"))
        .unwrap();
    let second = session_store
        .create_session(tempdir.path().join("dockyard-b"))
        .unwrap();
    session_store
        .rename_session(first.id, "Dockyard review".to_string())
        .unwrap();
    session_store
        .rename_session(second.id, "Dockyard follow-up".to_string())
        .unwrap();

    let mut state = AppState::new(
        PufferConfig::default(),
        tempdir.path().join("primary"),
        primary.clone(),
    );
    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut ProviderRegistry::new(),
        &mut AuthStore::default(),
        &session_store,
        "/resume dockyard",
    )
    .unwrap();

    assert_eq!(state.session.id, primary.id);
    assert!(state
        .transcript
        .last()
        .unwrap()
        .text
        .contains("Found 2 sessions matching `dockyard`"));
}

#[test]
fn tag_toggles_session_tags() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store
        .create_session(tempdir.path().join("primary"))
        .unwrap();
    let mut state = AppState::new(
        PufferConfig::default(),
        tempdir.path().join("primary"),
        session,
    );

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut ProviderRegistry::new(),
        &mut AuthStore::default(),
        &session_store,
        "/tag bugfix",
    )
    .unwrap();
    assert_eq!(state.session.tags, vec!["bugfix".to_string()]);

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut ProviderRegistry::new(),
        &mut AuthStore::default(),
        &session_store,
        "/tag bugfix",
    )
    .unwrap();
    assert!(state.session.tags.is_empty());
}
