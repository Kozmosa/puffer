use super::*;
use crate::RenderedMessage;
use puffer_config::{ensure_workspace_dirs, ConfigPaths, MascotConfig, PufferConfig, UiConfig};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::LoadedResources;
use puffer_session_store::{SessionMetadata, SessionStore};
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
fn command_registry_contains_review_usage_and_resume_alias() {
    let commands = supported_commands();
    assert!(find_command(&commands, "review").is_some());
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
            theme: "puffer".to_string(),
            mascot: MascotConfig {
                id: "clawd".to_string(),
                display_name: "Clawd".to_string(),
                enabled: true,
            },
            ui: UiConfig {
                no_alt_screen: false,
                tmux_golden_mode: false,
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
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/theme harbor",
    )
    .unwrap();

    let record = session_store.load_session(session.id).unwrap();
    assert!(matches!(
        record.events.last(),
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
                sandbox_mode: "workspace-write".to_string(),
                remote_name: None,
                remote_environment: None,
                statusline_enabled: true,
                working_dirs: vec![tempdir.path().join("secondary").display().to_string()],
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
        &ProviderRegistry::new(),
        &AuthStore::default(),
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
fn branch_forks_and_switches_current_session() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store
        .create_session(tempdir.path().to_path_buf())
        .unwrap();
    let original_id = session.id;
    let mut state = AppState::new(
        PufferConfig::default(),
        tempdir.path().to_path_buf(),
        session,
    );

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/branch drydock",
    )
    .unwrap();

    assert_ne!(state.session.id, original_id);
    assert_eq!(state.session.parent_session_id, Some(original_id));
    assert_eq!(state.session.display_name.as_deref(), Some("drydock"));
}

#[test]
fn memory_command_updates_session_metadata() {
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
        session,
    );

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/memory note keep shipping",
    )
    .unwrap();
    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/memory tag add parity",
    )
    .unwrap();
    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/memory slug shipyard",
    )
    .unwrap();

    let record = session_store.load_session(state.session.id).unwrap();
    assert_eq!(state.session.note.as_deref(), Some("keep shipping"));
    assert_eq!(state.session.slug.as_deref(), Some("shipyard"));
    assert!(state.session.tags.iter().any(|tag| tag == "parity"));
    assert_eq!(record.metadata.note.as_deref(), Some("keep shipping"));
    assert_eq!(record.metadata.slug.as_deref(), Some("shipyard"));
    assert!(record.metadata.tags.iter().any(|tag| tag == "parity"));
}

#[test]
fn config_command_writes_workspace_config() {
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
        session,
    );

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/config set theme harbor",
    )
    .unwrap();

    let config_text = std::fs::read_to_string(paths.workspace_config_file()).unwrap();
    assert!(config_text.contains("theme = \"harbor\""));
}

#[test]
fn keybindings_command_creates_workspace_file() {
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
        session,
    );

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/keybindings",
    )
    .unwrap();

    let keybindings_path = paths.workspace_config_dir.join("keybindings.toml");
    let keybindings = std::fs::read_to_string(keybindings_path).unwrap();
    assert!(keybindings.contains("submit = \"enter\""));
}

#[test]
fn hooks_command_creates_workspace_file() {
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
        session,
    );

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/hooks",
    )
    .unwrap();

    let hooks_path = paths.workspace_config_dir.join("hooks.yaml");
    let hooks = std::fs::read_to_string(hooks_path).unwrap();
    assert!(hooks.contains("on_tool_start"));
}

#[test]
fn agents_command_creates_workspace_file() {
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
        session,
    );

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/agents",
    )
    .unwrap();

    let agents_path = paths.workspace_config_dir.join("agents.yaml");
    let agents = std::fs::read_to_string(agents_path).unwrap();
    assert!(agents.contains("agents:"));
    assert!(agents.contains("id: default"));
}

#[test]
fn prompt_commands_append_user_message_and_surface_runtime_failures() {
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
        session,
    );

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/review",
    )
    .unwrap();

    assert!(matches!(
        state.transcript.first(),
        Some(RenderedMessage {
            role: MessageRole::User,
            ..
        })
    ));
    assert!(matches!(
        state.transcript.last(),
        Some(RenderedMessage {
            role: MessageRole::System,
            text,
        }) if text.contains("Prompt command /review failed")
    ));
}

#[test]
fn tasks_command_reports_recorded_runtime_tasks() {
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
        session,
    );
    state.record_task("bash", "printf hi", true);

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/tasks",
    )
    .unwrap();

    assert!(matches!(
        state.transcript.last(),
        Some(RenderedMessage {
            role: MessageRole::System,
            text,
        }) if text.contains("bash") && text.contains("completed")
    ));
}

#[test]
fn cost_command_reports_runtime_summary() {
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
        session,
    );
    state.push_message(MessageRole::User, "review this");
    state.push_message(MessageRole::Assistant, "done");
    state.push_message(MessageRole::System, "Tool bash [ok]\ninput: printf hi\nhi");
    state.record_task("bash", "printf hi", true);

    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/cost",
    )
    .unwrap();

    assert!(matches!(
        state.transcript.last(),
        Some(RenderedMessage {
            role: MessageRole::System,
            text,
        }) if text.contains("assistant_messages=1")
            && text.contains("tool_invocations=1")
            && text.contains("recorded_tasks=1")
    ));
}

#[test]
fn reload_plugins_reports_resource_counts() {
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
        session,
    );
    let resources = LoadedResources {
        plugins: vec![LoadedItem {
            value: puffer_resources::PluginSpec {
                id: "git".to_string(),
                display_name: "Git".to_string(),
                description: "Git helpers".to_string(),
                commands: Vec::new(),
                skills: Vec::new(),
                mcp_servers: Vec::new(),
            },
            source_info: puffer_resources::SourceInfo {
                path: PathBuf::from("plugins/git.yaml"),
                kind: puffer_resources::SourceKind::Builtin,
            },
        }],
        skills: vec![LoadedItem {
            value: puffer_resources::SkillSpec {
                name: "reviewer".to_string(),
                description: "review".to_string(),
                content: "review".to_string(),
                disable_model_invocation: false,
            },
            source_info: puffer_resources::SourceInfo {
                path: PathBuf::from("skills/reviewer.md"),
                kind: puffer_resources::SourceKind::Builtin,
            },
        }],
        mcp_servers: vec![LoadedItem {
            value: puffer_resources::McpServerSpec {
                id: "docs".to_string(),
                display_name: "Docs".to_string(),
                transport: "stdio".to_string(),
                endpoint: String::new(),
                target: "docs".to_string(),
                description: "docs".to_string(),
            },
            source_info: puffer_resources::SourceInfo {
                path: PathBuf::from("mcp/docs.yaml"),
                kind: puffer_resources::SourceKind::Builtin,
            },
        }],
        ..LoadedResources::default()
    };

    dispatch_command(
        &mut state,
        &supported_commands(),
        &resources,
        &ProviderRegistry::new(),
        &AuthStore::default(),
        &session_store,
        "/reload-plugins",
    )
    .unwrap();

    assert!(matches!(
        state.transcript.last(),
        Some(RenderedMessage {
            role: MessageRole::System,
            text,
        }) if text.contains("plugins=1")
            && text.contains("skills=1")
            && text.contains("mcp_servers=1")
    ));
}
