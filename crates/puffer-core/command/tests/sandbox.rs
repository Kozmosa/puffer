use super::*;

#[test]
fn sandbox_command_writes_workspace_sandbox_file() {
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
        &mut ProviderRegistry::new(),
        &mut AuthStore::default(),
        &session_store,
        "/sandbox read-only",
    )
    .unwrap();
    dispatch_command(
        &mut state,
        &supported_commands(),
        &LoadedResources::default(),
        &mut ProviderRegistry::new(),
        &mut AuthStore::default(),
        &session_store,
        "/sandbox exclude \"rm -rf\"",
    )
    .unwrap();

    let sandbox_path = paths.workspace_config_dir.join("sandbox.toml");
    let sandbox = std::fs::read_to_string(sandbox_path).unwrap();
    assert!(sandbox.contains("mode = \"read-only\""));
    assert!(sandbox.contains("excluded_commands = [\"rm -rf\"]"));
    assert_eq!(state.sandbox_mode, "read-only");
}
