use super::flow_loop::*;
use super::*;
use crate::state::LoopKind;
use puffer_config::{ensure_workspace_dirs, save_user_config, ConfigPaths, PufferConfig};
use puffer_session_store::SessionMetadata;
use std::ffi::OsString;
use std::sync::{Mutex, OnceLock};
use tempfile::tempdir;

fn sample_state(session: SessionMetadata, cwd: &Path) -> AppState {
    AppState::new(PufferConfig::default(), cwd.to_path_buf(), session)
}

fn puffer_home_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct ScopedPufferHome {
    old_home: Option<OsString>,
}

impl ScopedPufferHome {
    fn set(path: &Path) -> Self {
        let old_home = std::env::var_os("PUFFER_HOME");
        std::env::set_var("PUFFER_HOME", path);
        Self { old_home }
    }
}

impl Drop for ScopedPufferHome {
    fn drop(&mut self) {
        if let Some(value) = self.old_home.take() {
            std::env::set_var("PUFFER_HOME", value);
        } else {
            std::env::remove_var("PUFFER_HOME");
        }
    }
}

struct ScopedPromptSubmitEnv {
    _tempdir: tempfile::TempDir,
    _home: ScopedPufferHome,
    auth_path: std::path::PathBuf,
    session_store: SessionStore,
    state: AppState,
}

fn with_scoped_prompt_submit_env<R>(f: impl FnOnce(ScopedPromptSubmitEnv) -> R) -> R {
    let _lock = puffer_home_lock().lock().unwrap();
    let tempdir = tempdir().unwrap();
    let home = tempdir.path().join("home");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let _home = ScopedPufferHome::set(&home);
    let paths = ConfigPaths::discover(&workspace);
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store.create_session(workspace.clone()).unwrap();
    let state = sample_state(session, &workspace);
    let auth_path = paths.user_config_dir.join("auth.json");

    f(ScopedPromptSubmitEnv {
        _tempdir: tempdir,
        _home,
        auth_path,
        session_store,
        state,
    })
}

#[test]
fn provider_prompt_detection_matches_interactive_surface() {
    assert!(is_provider_prompt_input("henlo"));
    assert!(is_provider_prompt_input(" review this diff "));
    assert!(!is_provider_prompt_input(""));
    assert!(!is_provider_prompt_input("/help"));
    assert!(!is_provider_prompt_input("!pwd"));
    assert!(!is_provider_prompt_input("login openai"));
    assert!(!is_provider_prompt_input("/logout"));
}

#[test]
fn handle_prompt_submit_starts_async_provider_turn_and_polls_result() {
    with_scoped_prompt_submit_env(|mut env| {
        let mut resources = LoadedResources::default();
        let mut providers = ProviderRegistry::new();
        let mut auth_store = AuthStore::default();
        let mut tui = TuiState::default();

        handle_prompt_submit(
            &mut env.state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &env.auth_path,
            &env.session_store,
            &mut tui,
            "henlo".to_string(),
            true,
        )
        .unwrap();

        assert!(tui.has_pending_submit());
        assert!(matches!(env.state.transcript.first(), Some(message) if message.text == "henlo"));

        let mut completed = false;
        for _ in 0..20 {
            if poll_pending_submit(
                &mut env.state,
                &mut auth_store,
                &env.auth_path,
                &env.session_store,
                &mut tui,
            )
            .unwrap()
            {
                completed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(completed);
        assert!(!tui.has_pending_submit());
        assert!(env.state.transcript.iter().any(|message| {
            message.role == MessageRole::System
                && message.text.starts_with("Provider request failed:")
        }));
    });
}

#[test]
fn handle_prompt_submit_queues_prompt_while_turn_is_running() {
    with_scoped_prompt_submit_env(|mut env| {
        let mut resources = LoadedResources::default();
        let mut providers = ProviderRegistry::new();
        let mut auth_store = AuthStore::default();
        let mut tui = TuiState::default();

        handle_prompt_submit(
            &mut env.state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &env.auth_path,
            &env.session_store,
            &mut tui,
            "first".to_string(),
            true,
        )
        .unwrap();
        handle_prompt_submit(
            &mut env.state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &env.auth_path,
            &env.session_store,
            &mut tui,
            "second".to_string(),
            true,
        )
        .unwrap();

        assert!(tui.has_pending_submit());
        assert_eq!(tui.queued_prompts.len(), 1);
        assert_eq!(
            tui.queued_prompts.front().map(String::as_str),
            Some("second")
        );
        assert!(matches!(env.state.transcript.first(), Some(message) if message.text == "first"));
    });
}

#[test]
fn slash_plan_with_arguments_starts_async_turn_after_local_handling() {
    with_scoped_prompt_submit_env(|mut env| {
        let paths = ConfigPaths::discover(&env.state.cwd);
        let mut config = PufferConfig::default();
        config.default_provider = Some("openai".to_string());
        config.default_model = Some("openai/gpt-5".to_string());
        save_user_config(&paths, &config).unwrap();
        env.state.config = config;
        env.state.current_provider = Some("openai".to_string());
        env.state.current_model = Some("openai/gpt-5".to_string());

        let mut resources = LoadedResources::default();
        let mut providers = ProviderRegistry::new();
        providers.register(puffer_provider_registry::ProviderDescriptor {
            id: "openai".to_string(),
            display_name: "OpenAI".to_string(),
            base_url: "https://api.openai.com".to_string(),
            default_api: "responses".to_string(),
            auth_modes: vec![
                puffer_provider_registry::AuthMode::ApiKey,
                puffer_provider_registry::AuthMode::OAuth,
            ],
            headers: Default::default(),
            query_params: Default::default(),
            discovery: None,
            models: vec![puffer_provider_registry::ModelDescriptor {
                id: "gpt-5".to_string(),
                display_name: "GPT-5".to_string(),
                provider: "openai".to_string(),
                api: "responses".to_string(),
                context_window: 200_000,
                max_output_tokens: 8_192,
                supports_reasoning: true,
                compat: None,
                input: vec![puffer_provider_registry::Modality::Text],
                cost: None,
            }],
            chat_completions_path: None,
        });
        let mut auth_store = AuthStore::default();
        auth_store.set_api_key("openai", "sk-openai");
        let mut tui = TuiState::default();

        handle_prompt_submit(
            &mut env.state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &env.auth_path,
            &env.session_store,
            &mut tui,
            "/plan stabilize slash-command parity".to_string(),
            true,
        )
        .unwrap();
        submit_queued_prompt_if_ready(
            &mut env.state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &env.auth_path,
            &env.session_store,
            &mut tui,
            true,
        )
        .unwrap();

        assert!(env.state.plan_mode);
        assert!(tui.has_pending_submit());
        assert!(env
            .state
            .transcript
            .iter()
            .any(|message| message.text == "Enabled plan mode"));
        assert!(env.state.transcript.iter().any(|message| {
            message.role == MessageRole::User && message.text == "stabilize slash-command parity"
        }));
    });
}

#[test]
fn cancel_pending_submit_records_interrupt_and_starts_next_queued_prompt() {
    with_scoped_prompt_submit_env(|mut env| {
        let mut resources = LoadedResources::default();
        let mut providers = ProviderRegistry::new();
        let mut auth_store = AuthStore::default();
        let mut tui = TuiState::default();

        handle_prompt_submit(
            &mut env.state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &env.auth_path,
            &env.session_store,
            &mut tui,
            "first".to_string(),
            true,
        )
        .unwrap();
        handle_prompt_submit(
            &mut env.state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &env.auth_path,
            &env.session_store,
            &mut tui,
            "second".to_string(),
            true,
        )
        .unwrap();

        assert!(cancel_pending_submit(&mut env.state, &env.session_store, &mut tui).unwrap());
        assert!(!tui.has_pending_submit());
        assert!(env.state.transcript.iter().any(|message| {
            message.role == MessageRole::System && message.text == "Interrupted by user."
        }));

        assert!(submit_next_queued_prompt(
            &mut env.state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &env.auth_path,
            &env.session_store,
            &mut tui,
            true,
        )
        .unwrap());
        assert!(tui.has_pending_submit());
        assert!(tui.queued_prompts.is_empty());
        assert!(env
            .state
            .transcript
            .iter()
            .any(|message| { message.role == MessageRole::User && message.text == "second" }));
    });
}

#[test]
fn poll_pending_submit_preserves_browser_typed_grant_round_trip() {
    with_scoped_prompt_submit_env(|mut env| {
        let mut auth_store = AuthStore::default();
        let mut tui = TuiState::default();

        let browser = puffer_tools::ToolDefinition {
            id: "Browser".to_string(),
            name: "Browser".to_string(),
            description: String::new(),
            handler: String::new(),
            aliases: Vec::new(),
            handler_args: Vec::new(),
            kind: puffer_tools::ToolKind::Custom,
            input_schema: puffer_tools::ToolInputSchema::default(),
            metadata: puffer_tools::ToolMetadata::default(),
            policy: puffer_tools::ToolPolicyHints::default(),
            shared_lib: None,
            enabled_if: None,
            display: puffer_tools::ToolDisplayHints::default(),
        };
        env.state.allow_permission_for_tool_call(
            &browser,
            &serde_json::json!({"action": "evaluate", "script": "document.title"}),
        );
        let expected_permission_state = env.state.session_permission_state().clone();

        let (event_tx, event_rx) = std::sync::mpsc::channel();
        event_tx
            .send(PendingSubmitEvent::Finished(PendingSubmitResult {
                outcome: Err("cancelled".to_string()),
                auth_store: auth_store.clone(),
                session_permission_state: expected_permission_state.clone(),
            }))
            .unwrap();

        tui.pending_submit = Some(PendingSubmit {
            prompt: "hi".to_string(),
            receiver: event_rx,
            rendered_tool_invocations: 0,
            pending_tool_calls: Vec::new(),
            started_at: std::time::Instant::now(),
            thinking_active: false,
            status_hint: None,
            cancel: puffer_core::CancelToken::new(),
        });

        let completed = poll_pending_submit(
            &mut env.state,
            &mut auth_store,
            &env.auth_path,
            &env.session_store,
            &mut tui,
        )
        .unwrap();

        assert!(completed);
        assert_eq!(env.state.session_permission_state(), &expected_permission_state);
        assert!(env.state.session_permission_state().has_browser_grant());
    });
}

#[test]
fn handle_prompt_submit_preserves_typed_tool_grant_before_worker_round_trip() {
    with_scoped_prompt_submit_env(|mut env| {
        let bash = puffer_tools::ToolDefinition {
            id: "Bash".to_string(),
            name: "Bash".to_string(),
            description: String::new(),
            handler: String::new(),
            aliases: Vec::new(),
            handler_args: Vec::new(),
            kind: puffer_tools::ToolKind::Custom,
            input_schema: puffer_tools::ToolInputSchema::default(),
            metadata: puffer_tools::ToolMetadata::default(),
            policy: puffer_tools::ToolPolicyHints::default(),
            shared_lib: None,
            enabled_if: None,
            display: puffer_tools::ToolDisplayHints::default(),
        };
        env.state
            .allow_permission_for_tool_call(&bash, &serde_json::Value::Null);
        let expected_permission_state = env.state.session_permission_state().clone();
        let mut resources = LoadedResources::default();
        let mut providers = ProviderRegistry::new();
        let mut auth_store = AuthStore::default();
        let mut tui = TuiState::default();

        handle_prompt_submit(
            &mut env.state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &env.auth_path,
            &env.session_store,
            &mut tui,
            "henlo".to_string(),
            true,
        )
        .unwrap();

        assert_eq!(env.state.session_permission_state(), &expected_permission_state);
        assert!(tui.has_pending_submit());

        let mut completed = false;
        for _ in 0..20 {
            if poll_pending_submit(
                &mut env.state,
                &mut auth_store,
                &env.auth_path,
                &env.session_store,
                &mut tui,
            )
            .unwrap()
            {
                completed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(completed);
        assert_eq!(env.state.session_permission_state(), &expected_permission_state);
    });
}

#[test]
fn handle_prompt_submit_preserves_typed_allow_all_before_worker_round_trip() {
    with_scoped_prompt_submit_env(|mut env| {
        env.state.grant_all_tools_for_session();
        let expected_permission_state = env.state.session_permission_state().clone();
        let mut resources = LoadedResources::default();
        let mut providers = ProviderRegistry::new();
        let mut auth_store = AuthStore::default();
        let mut tui = TuiState::default();

        handle_prompt_submit(
            &mut env.state,
            &mut resources,
            &mut providers,
            &mut auth_store,
            &env.auth_path,
            &env.session_store,
            &mut tui,
            "henlo".to_string(),
            true,
        )
        .unwrap();

        assert_eq!(env.state.session_permission_state(), &expected_permission_state);
        assert!(env.state.session_permission_state().allow_all_tools());
        assert!(tui.has_pending_submit());

        let mut completed = false;
        for _ in 0..20 {
            if poll_pending_submit(
                &mut env.state,
                &mut auth_store,
                &env.auth_path,
                &env.session_store,
                &mut tui,
            )
            .unwrap()
            {
                completed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(completed);
        assert_eq!(env.state.session_permission_state(), &expected_permission_state);
        assert!(env.state.session_permission_state().allow_all_tools());
    });
}

// ---------------------------------------------------------------------------
// Loop / Maximize / Minimize tests
// ---------------------------------------------------------------------------

#[test]
fn parse_loop_args_extracts_interval_and_prompt() {
    let (d, p) = parse_loop_args("5m check deploy");
    assert_eq!(d, std::time::Duration::from_secs(300));
    assert_eq!(p, "check deploy");

    let (d, p) = parse_loop_args("30s ping server");
    assert_eq!(d, std::time::Duration::from_secs(30));
    assert_eq!(p, "ping server");

    let (d, p) = parse_loop_args("2h run maintenance");
    assert_eq!(d, std::time::Duration::from_secs(7200));
    assert_eq!(p, "run maintenance");

    // No interval → defaults to 10 minutes, whole input is prompt.
    let (d, p) = parse_loop_args("check deploy");
    assert_eq!(d, std::time::Duration::from_secs(600));
    assert_eq!(p, "check deploy");
}

#[test]
fn parse_duration_handles_all_suffixes() {
    assert_eq!(
        parse_duration("10s"),
        Some(std::time::Duration::from_secs(10))
    );
    assert_eq!(
        parse_duration("5m"),
        Some(std::time::Duration::from_secs(300))
    );
    assert_eq!(
        parse_duration("1h"),
        Some(std::time::Duration::from_secs(3600))
    );
    assert_eq!(
        parse_duration("2d"),
        Some(std::time::Duration::from_secs(172800))
    );
    assert_eq!(parse_duration("abc"), None);
    assert_eq!(parse_duration(""), None);
}

#[test]
fn extract_metric_value_parses_marker() {
    let text = "I improved the test suite.\n[[METRIC:accuracy=0.85]]\nDone.";
    assert_eq!(extract_metric_value(text, "accuracy"), Some(0.85));

    let text = "[[METRIC:latency = 12.5 ]]";
    assert_eq!(extract_metric_value(text, "latency"), Some(12.5));

    let text = "No metric here";
    assert_eq!(extract_metric_value(text, "accuracy"), None);
}

#[test]
fn has_converged_detects_plateau() {
    assert!(!has_converged(&[]));
    assert!(!has_converged(&[1.0, 2.0]));
    assert!(!has_converged(&[1.0, 2.0, 3.0]));
    assert!(has_converged(&[3.0, 3.0, 3.0]));
    assert!(has_converged(&[1.0, 2.0, 3.0, 3.0, 3.0]));
}

#[test]
fn build_optimization_prompt_includes_context() {
    let prompt = build_optimization_prompt("fix tests", "accuracy", true, 3, &[0.5, 0.7]);
    assert!(prompt.contains("maximize"));
    assert!(prompt.contains("accuracy"));
    assert!(prompt.contains("iteration 3/"));
    assert!(prompt.contains("0.5000"));
    assert!(prompt.contains("0.7000"));
    assert!(prompt.contains("fix tests"));
    assert!(prompt.contains("[[METRIC:accuracy="));
    // User prompt should come before optimization context
    let user_pos = prompt.find("fix tests").unwrap();
    let ctx_pos = prompt.find("Optimization context").unwrap();
    assert!(
        user_pos < ctx_pos,
        "user prompt should precede optimization context"
    );
}

#[test]
fn try_handle_loop_command_creates_loop_state() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store
        .create_session(tempdir.path().to_path_buf())
        .unwrap();
    let mut state = sample_state(session, tempdir.path());
    let mut tui = TuiState::default();

    // Non-loop command returns false.
    assert!(!try_handle_loop_command(&mut state, &session_store, &mut tui, "/help").unwrap());

    // Loop command creates state and enqueues prompt.
    assert!(
        try_handle_loop_command(&mut state, &session_store, &mut tui, "/loop 10s echo hi").unwrap()
    );
    assert!(tui.active_loop.is_some());
    let ls = tui.active_loop.as_ref().unwrap();
    assert!(matches!(ls.kind, LoopKind::Loop));
    assert_eq!(ls.prompt, "echo hi");
    assert_eq!(ls.interval, Some(std::time::Duration::from_secs(10)));
    assert_eq!(tui.queued_prompts.len(), 1);
}

#[test]
fn try_handle_loop_command_creates_maximize_state() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store
        .create_session(tempdir.path().to_path_buf())
        .unwrap();
    let mut state = sample_state(session, tempdir.path());
    let mut tui = TuiState::default();

    assert!(try_handle_loop_command(
        &mut state,
        &session_store,
        &mut tui,
        "/maximize accuracy run bench"
    )
    .unwrap());
    assert!(tui.active_loop.is_some());
    let ls = tui.active_loop.as_ref().unwrap();
    assert!(matches!(ls.kind, LoopKind::Maximize(ref m) if m == "accuracy"));
    assert_eq!(ls.prompt, "run bench");
    assert_eq!(tui.queued_prompts.len(), 1);
    let enqueued = &tui.queued_prompts[0];
    assert!(enqueued.contains("maximize"));
    assert!(enqueued.contains("[[METRIC:accuracy="));
}

#[test]
fn maybe_apply_requested_reload_swallows_parse_errors_as_system_message() {
    // Regression: a malformed YAML file under `.puffer/resources/`
    // (e.g. user mid-edit, atomic-rename save catching the file in a
    // transient invalid state) must not propagate out of
    // `maybe_apply_requested_reload`. The watcher fires after every
    // save; a single bad save would otherwise kill the TUI and discard
    // the in-memory transcript. The reload error should surface as a
    // system message instead.
    let tempdir = tempdir().unwrap();
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let paths = ConfigPaths {
        workspace_root: workspace.clone(),
        workspace_config_dir: workspace.join(".puffer"),
        user_config_dir: tempdir.path().join(".home/.puffer"),
        builtin_resources_dir: tempdir.path().join("builtin-resources"),
    };
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store.create_session(workspace.clone()).unwrap();
    let mut state = sample_state(session, &workspace);
    let mut resources = LoadedResources::default();
    let mut providers = ProviderRegistry::new();
    let auth_store = AuthStore::default();

    // Drop a syntactically broken MCP manifest into the watched dir.
    let mcp_dir = paths.workspace_config_dir.join("resources/mcp_servers");
    std::fs::create_dir_all(&mcp_dir).unwrap();
    std::fs::write(
        mcp_dir.join("busted.yaml"),
        "id: broken\nthis is: : invalid: yaml\n  - structure\n",
    )
    .unwrap();

    // Simulate a watcher-driven reload request.
    state
        .reload_signal()
        .store(true, std::sync::atomic::Ordering::Release);

    // Must NOT propagate — the TUI loop calls this with `?` and a
    // returned `Err` would crash `run_app` and lose the session.
    let result = maybe_apply_requested_reload(
        &mut state,
        &mut resources,
        &mut providers,
        &auth_store,
        &session_store,
    );
    assert!(
        result.is_ok(),
        "reload parse error must be swallowed, got: {result:?}"
    );

    // The transcript should now contain a system message describing
    // the failure so the user knows what happened.
    let last = state.transcript.last().expect("system message appended");
    assert!(matches!(last.role, MessageRole::System));
    assert!(
        last.text.contains("Resource hot-reload failed"),
        "expected reload-failure system message, got: {}",
        last.text
    );

    // And the signal must have been consumed so we don't spin trying
    // to reload the same broken file every loop tick.
    assert!(!state.take_reload_request());
}

#[test]
fn maybe_apply_requested_reload_no_op_when_no_signal_pending() {
    // Sanity: when neither the in-loop flag nor the watcher signal is
    // set, the reload helper is a cheap no-op and doesn't touch the
    // transcript. This is the dominant code path on every TUI loop
    // tick — must stay free of side effects.
    with_scoped_prompt_submit_env(|mut env| {
        let initial_len = env.state.transcript.len();
        let mut resources = LoadedResources::default();
        let mut providers = ProviderRegistry::new();
        let auth_store = AuthStore::default();

        maybe_apply_requested_reload(
            &mut env.state,
            &mut resources,
            &mut providers,
            &auth_store,
            &env.session_store,
        )
        .unwrap();
        assert_eq!(env.state.transcript.len(), initial_len);
    });
}

#[test]
fn stop_command_clears_active_loop() {
    let tempdir = tempdir().unwrap();
    let paths = ConfigPaths::discover(tempdir.path());
    ensure_workspace_dirs(&paths).unwrap();
    let session_store = SessionStore::from_paths(&paths).unwrap();
    let session = session_store
        .create_session(tempdir.path().to_path_buf())
        .unwrap();
    let mut state = sample_state(session, tempdir.path());
    let mut tui = TuiState::default();

    try_handle_loop_command(&mut state, &session_store, &mut tui, "/loop 10s echo hi").unwrap();
    assert!(tui.active_loop.is_some());

    try_handle_loop_command(&mut state, &session_store, &mut tui, "/loop stop").unwrap();
    assert!(tui.active_loop.is_none());
}
