use super::*;
use crate::runtime::{
    with_user_question_prompt_handler, UserQuestionPromptRequest, UserQuestionPromptResponse,
};
use puffer_config::PufferConfig;
use puffer_session_store::SessionMetadata;
use serde_json::Map;
use std::sync::{Arc, Mutex};

#[test]
fn first_http_url_extracts_clean_url() {
    let line = "  https://open.feishu.cn/page/cli?user_code=2FVM-NAV3&from=cli";
    assert_eq!(
        first_http_url(line).as_deref(),
        Some("https://open.feishu.cn/page/cli?user_code=2FVM-NAV3&from=cli")
    );
    assert_eq!(first_http_url("no url here"), None);
    // Trailing punctuation from a surrounding log line is trimmed.
    assert_eq!(
        first_http_url("see (https://x.test/a).").as_deref(),
        Some("https://x.test/a")
    );
}

#[test]
fn build_auth_preview_includes_url_and_optional_qr() {
    let with_qr = build_auth_preview("https://x.test/auth", Some("/tmp/lark-qr.png"));
    assert!(with_qr.contains("https://x.test/auth"));
    assert!(with_qr.contains("/tmp/lark-qr.png"));
    let without_qr = build_auth_preview("https://x.test/auth", None);
    assert!(without_qr.contains("https://x.test/auth"));
    assert!(!without_qr.contains("QR image"));
}

#[test]
fn lark_cli_status_detects_missing_app_config() {
    let missing = json!({
        "ok": false,
        "error": {
            "type": "config",
            "subtype": "not_configured",
            "message": "not configured"
        }
    });
    assert!(lark_cli_status_is_config_not_configured(&missing));

    let logged_out = json!({
        "ok": false,
        "error": {
            "type": "auth",
            "subtype": "not_logged_in",
            "message": "not logged in"
        }
    });
    assert!(!lark_cli_status_is_config_not_configured(&logged_out));
}

#[test]
fn lark_cli_status_payload_reads_stderr_failures() {
    use std::os::unix::process::ExitStatusExt;

    let output = std::process::Output {
        status: std::process::ExitStatus::from_raw(3 << 8),
        stdout: Vec::new(),
        stderr: br#"{"ok":false,"error":{"type":"config","subtype":"not_configured"}}"#.to_vec(),
    };
    let payload = lark_cli_status_payload(&output).expect("status payload");

    assert!(lark_cli_status_is_config_not_configured(&payload));
}

#[test]
fn lark_cli_config_init_new_args_include_selected_brand() {
    assert_eq!(
        lark_cli_config_init_new_args("lark"),
        ["config", "init", "--new", "--brand", "lark"]
    );
    assert_eq!(
        lark_cli_config_init_new_args("feishu"),
        ["config", "init", "--new", "--brand", "feishu"]
    );
}

#[cfg(unix)]
#[test]
fn terminate_lark_cli_child_signals_descendant_wrapper_process() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let tempdir = tempfile::tempdir().unwrap();
    let inner = tempdir.path().join("inner.sh");
    let wrapper = tempdir.path().join("wrapper.sh");
    let marker = tempdir.path().join("inner-terminated");
    let pid_file = tempdir.path().join("inner.pid");
    let ready_file = tempdir.path().join("inner-ready");

    std::fs::write(
            &inner,
            "#!/bin/sh\necho $$ > \"$2\"\ntrap 'touch \"$1\"; exit 0' TERM\ntouch \"$3\"\nwhile true; do sleep 1; done\n",
        )
        .unwrap();
    std::fs::write(&wrapper, "#!/bin/sh\n\"$1\" \"$2\" \"$3\" \"$4\"\n").unwrap();
    for script in [&inner, &wrapper] {
        let mut permissions = std::fs::metadata(script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(script, permissions).unwrap();
    }

    let mut command = lark_cli_command(wrapper.to_str().unwrap());
    let mut child = command
        .arg(inner.to_str().unwrap())
        .arg(marker.to_str().unwrap())
        .arg(pid_file.to_str().unwrap())
        .arg(ready_file.to_str().unwrap())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_until(Duration::from_secs(2), || ready_file.exists());

    terminate_lark_cli_child(&mut child);

    wait_until(Duration::from_secs(2), || marker.exists());

    fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(predicate(), "condition timed out");
    }
}

#[test]
fn lark_brand_cli_value_accepts_display_labels() {
    assert_eq!(lark_brand_cli_value("Lark"), "lark");
    assert_eq!(lark_brand_cli_value("Feishu"), "feishu");
    assert_eq!(lark_brand_cli_value(" feishu "), "feishu");
}

fn temp_state() -> AppState {
    let tempdir = tempfile::tempdir().unwrap();
    let cwd = tempdir.keep();
    let session = SessionMetadata {
        id: uuid::Uuid::nil(),
        display_name: None,
        generated_title: None,
        cwd: cwd.clone(),
        created_at_ms: 0,
        updated_at_ms: 0,
        parent_session_id: None,
        slug: None,
        tags: Vec::new(),
        note: None,
    };
    AppState::new(PufferConfig::default(), cwd, session)
}

fn connector_config(state: &AppState) -> String {
    std::fs::read_to_string(state.cwd.join(".puffer/connectors.toml")).expect("config")
}

fn answer_connect_question(request: &UserQuestionPromptRequest) -> UserQuestionPromptResponse {
    let question = request.questions[0]["question"]
        .as_str()
        .expect("question text")
        .to_string();
    let answer = match question.as_str() {
        "What Telegram bot token should Puffer use?" => "telegram-token",
        other => panic!("unexpected question: {other}"),
    };
    UserQuestionPromptResponse {
        answers: Map::from_iter([(question, json!(answer))]),
        annotations: Map::new(),
    }
}

#[test]
fn parse_target_uses_two_args_without_questions() {
    let mut state = temp_state();
    let resources = LoadedResources::default();

    let target = parse_or_ask_target(&mut state, &resources, "telegram-login telegram-user")
        .expect("target");

    assert_eq!(target.connector_slug, "telegram-login");
    assert_eq!(target.connection_name, "telegram-user");
}

#[test]
fn telegram_qr_approval_question_embeds_qr_markdown_in_question_body() {
    let question = telegram_qr_approval_question("tg://login?token=abc");

    assert!(
        question.starts_with("Approve this Telegram QR login URL from a logged-in Telegram app.")
    );
    assert!(question.contains("![Telegram QR code](data:image/svg+xml;base64,"));
    assert!(question.contains("tg://login?token=abc"));
}

#[test]
fn telegram_qr_wait_input_uses_short_retry_timeout() {
    let input = telegram_qr_wait_input("telegram-user");

    assert_eq!(input["action"], "login_qr_wait");
    assert_eq!(input["connection_slug"], "telegram-user");
    assert_eq!(input["timeout_seconds"], TELEGRAM_QR_APPROVAL_CHECK_SECONDS);
}

#[test]
fn parse_target_resolves_unique_connector_search_term() {
    let mut state = temp_state();
    let resources = LoadedResources::default();

    let target = parse_or_ask_target(&mut state, &resources, "matrix matrix-main").expect("target");

    assert_eq!(target.connector_slug, "matrix-bot");
    assert_eq!(target.connection_name, "matrix-main");
}

#[test]
fn parse_target_resolves_unique_action_search_term() {
    let mut state = temp_state();
    let resources = LoadedResources::default();

    let target = parse_or_ask_target(&mut state, &resources, "vote telegram-user").expect("target");

    assert_eq!(target.connector_slug, "telegram-login");
    assert_eq!(target.connection_name, "telegram-user");
}

#[test]
fn parse_target_uses_default_connection_name_for_connector_only() {
    let mut state = temp_state();
    let resources = LoadedResources::default();

    let target = parse_or_ask_target(&mut state, &resources, "email").expect("target");

    assert_eq!(target.connector_slug, "email");
    assert_eq!(target.connection_name, "email");
}

#[test]
fn parse_target_asks_for_connector_and_uses_default_connection_name() {
    let mut state = temp_state();
    let resources = LoadedResources::default();
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let request_log = Arc::clone(&requests);

    let target = with_user_question_prompt_handler(
        move |request| {
            let question = request.questions[0]["question"]
                .as_str()
                .expect("question text")
                .to_string();
            request_log.lock().unwrap().push(request.questions.clone());
            UserQuestionPromptResponse {
                answers: Map::from_iter([(question, json!("telegram-login"))]),
                annotations: Map::new(),
            }
        },
        || parse_or_ask_target(&mut state, &resources, ""),
    )
    .expect("target");

    assert_eq!(target.connector_slug, "telegram-login");
    assert_eq!(target.connection_name, "telegram-user");
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0][0]["type"], "choice");
    assert_eq!(requests[0][0]["searchable"], true);
    assert!(requests[0][0]["options"]
        .as_array()
        .is_some_and(|options| options
            .iter()
            .any(|option| option["label"] == "telegram-login")));
}

#[test]
fn execute_connect_flow_dispatches_telegram_bot_setup() {
    let mut state = temp_state();
    let resources = LoadedResources::default();

    let turn = with_user_question_prompt_handler(
        |request| answer_connect_question(&request),
        || execute_connect_flow(&mut state, &resources, "telegram-bot telegram-bot"),
    )
    .expect("connect turn");

    assert!(turn.assistant_text.contains("connector: telegram-bot"));
    assert!(turn.assistant_text.contains("run `puffer serve`"));
    let raw = connector_config(&state);
    assert!(raw.contains("[connectors.telegram]"));
    assert!(raw.contains("token = \"telegram-token\""));
}
