use super::{ask_choice, ask_choice_with_preview, ask_input, call_tool, summary, ConnectResult};
use crate::AppState;
use anyhow::{anyhow, bail, Context, Result};
use puffer_resources::LoadedResources;
use serde_json::{json, Value};

/// Connects a Lark account backed by the official `lark-cli`.
///
/// Auth is delegated to `lark-cli`: installs it on consent if missing, then runs
/// the device-flow login (showing the verification URL/QR) if not logged in, and
/// records a `ConnectionCreate`. Puffer never stores a Lark token.
pub(super) fn connect_lark_cli(
    state: &mut AppState,
    resources: &LoadedResources,
    connector_slug: &str,
    connection: &str,
) -> Result<ConnectResult> {
    let bin = lark_cli_bin();
    // If the CLI is missing, offer to install it (with consent) before auth.
    if !lark_cli_available(&bin) {
        ensure_lark_cli_installed(state, resources, &bin)?;
    }
    // Both connectors verify their lark-cli identity before connecting. The CLI
    // needs app configuration before any identity can log in, so recover that
    // first when auth status reports `config.not_configured`.
    let identity = lark_connector_identity(connector_slug);
    if !lark_cli_identity_ready(&bin, identity) {
        if lark_cli_config_not_configured(&bin) {
            ensure_lark_cli_app_configured(state, resources, &bin, identity)?;
        }
    }
    if !lark_cli_identity_ready(&bin, identity) {
        match identity {
            "user" => ensure_lark_cli_logged_in(state, resources, &bin)?,
            _ => ensure_lark_cli_app_configured(state, resources, &bin, identity)?,
        }
    }
    // Idempotent: re-running /connect with an existing connection name just
    // re-verifies auth (done above) instead of failing on a duplicate record.
    if lark_connection_exists(state, resources, connection)? {
        return Ok(summary(
            connector_slug,
            connection,
            "lark-cli auth status (existing connection)",
            &json!({ "status": "complete" }),
        ));
    }
    let output = call_tool(
        state,
        resources,
        "ConnectionCreate",
        json!({
            "slug": connection,
            "connector_slug": connector_slug,
            "description": "Lark connection backed by lark-cli, configured by /connect"
        }),
    )?;
    Ok(summary(
        connector_slug,
        connection,
        "lark-cli auth status",
        &output,
    ))
}

/// Returns the `lark-cli` binary name, honoring the `LARK_CLI_BIN` override.
fn lark_cli_bin() -> String {
    std::env::var("LARK_CLI_BIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "lark-cli".to_string())
}

/// Returns whether the `lark-cli` binary can be launched (i.e. is installed).
fn lark_cli_available(bin: &str) -> bool {
    std::process::Command::new(bin)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Runs `lark-cli <args>` capturing output, force-killing it after `timeout` so
/// a stalled device-flow login cannot hang the synchronous `/connect` turn.
fn lark_cli_output_timed(
    bin: &str,
    args: &[&str],
    timeout: std::time::Duration,
) -> Result<std::process::Output> {
    let mut command = lark_cli_command(bin);
    let mut child = command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn `{bin} {}`", args.join(" ")))?;
    let start = std::time::Instant::now();
    loop {
        if child.try_wait().context("wait for lark-cli")?.is_some() {
            return child.wait_with_output().context("collect lark-cli output");
        }
        if start.elapsed() >= timeout {
            terminate_lark_cli_child(&mut child);
            bail!("`{bin} {}` timed out", args.join(" "));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Builds a `lark-cli` command with platform-specific child-process cleanup settings.
pub(super) fn lark_cli_command(bin: &str) -> std::process::Command {
    let mut command = std::process::Command::new(bin);
    configure_lark_cli_child(&mut command);
    command
}

#[cfg(unix)]
fn configure_lark_cli_child(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;

    // lark-cli installed through npm is a Node wrapper that can spawn another
    // process. Put it in its own process group so timeout/cancel cleanup can
    // stop the wrapper and the real CLI process together.
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_lark_cli_child(_command: &mut std::process::Command) {}

/// Terminates a running `lark-cli` child and any process-group descendants.
pub(super) fn terminate_lark_cli_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        terminate_lark_cli_process_group(child.id(), libc::SIGTERM);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
        while std::time::Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        terminate_lark_cli_process_group(child.id(), libc::SIGKILL);
        let _ = child.wait();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(unix)]
fn terminate_lark_cli_process_group(pid: u32, signal: libc::c_int) {
    let pgid = pid as libc::pid_t;
    if pgid <= 0 {
        return;
    }
    // Ignore ESRCH: the process may have completed between try_wait and cleanup.
    unsafe {
        libc::killpg(pgid, signal);
    }
}

/// Returns whether a connection named `slug` is already registered, so a re-run
/// of `/connect` can short-circuit instead of failing on a duplicate.
fn lark_connection_exists(
    state: &mut AppState,
    resources: &LoadedResources,
    slug: &str,
) -> Result<bool> {
    let listed = call_tool(state, resources, "ConnectionList", json!({}))?;
    Ok(listed
        .get("connections")
        .and_then(Value::as_array)
        .is_some_and(|connections| {
            connections
                .iter()
                .any(|connection| connection.get("slug").and_then(Value::as_str) == Some(slug))
        }))
}

/// Maps a Lark connector slug to the `lark-cli` identity it authenticates as.
fn lark_connector_identity(connector_slug: &str) -> &'static str {
    match connector_slug {
        "lark-login" => "user",
        _ => "bot",
    }
}

/// Returns whether `lark-cli auth status` reports `identity` ("user"/"bot") as
/// available, parsing `identities.<identity>.available` and trusting a
/// successful exit when that field is absent (older `lark-cli`).
fn lark_cli_identity_ready(bin: &str, identity: &str) -> bool {
    let output = match std::process::Command::new(bin)
        .args(["auth", "status"])
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    match serde_json::from_slice::<Value>(&output.stdout) {
        Ok(status) => status
            .get("identities")
            .and_then(|identities| identities.get(identity))
            .and_then(|entry| entry.get("available"))
            .and_then(Value::as_bool)
            .unwrap_or(true),
        Err(_) => true,
    }
}

/// Returns whether `lark-cli auth status` reports that app configuration is
/// missing. `auth login` cannot start in this state, so `/connect lark-login`
/// must run app setup before the user OAuth flow.
fn lark_cli_config_not_configured(bin: &str) -> bool {
    let output = match std::process::Command::new(bin)
        .args(["auth", "status"])
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(_) => return false,
    };
    lark_cli_status_payload(&output)
        .as_ref()
        .is_some_and(lark_cli_status_is_config_not_configured)
}

/// Parses a JSON status payload from either stdout or stderr.
pub(super) fn lark_cli_status_payload(output: &std::process::Output) -> Option<Value> {
    serde_json::from_slice::<Value>(&output.stdout)
        .or_else(|_| serde_json::from_slice::<Value>(&output.stderr))
        .ok()
}

/// Returns whether a status payload reports missing app configuration.
pub(super) fn lark_cli_status_is_config_not_configured(payload: &Value) -> bool {
    payload.pointer("/error/type").and_then(Value::as_str) == Some("config")
        && payload.pointer("/error/subtype").and_then(Value::as_str) == Some("not_configured")
}

/// Generates a scannable PNG QR for `url` via `lark-cli auth qrcode --output`
/// (written to the temp dir, since the CLI only accepts a cwd-relative path) and
/// returns its absolute path, or `None` on failure. Callers point the user at
/// the saved file because terminal hosts cannot render an inline image.
fn lark_cli_png_qr(bin: &str, url: &str) -> Option<String> {
    let dir = std::env::temp_dir();
    let path = dir.join("lark-qr.png");
    // Drop any stale QR first so a no-op/failed run can't surface a prior URL's image.
    let _ = std::fs::remove_file(&path);
    let status = std::process::Command::new(bin)
        .args(["auth", "qrcode", url, "--output", "lark-qr.png"])
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    path.exists().then(|| path.to_string_lossy().into_owned())
}

/// Opens `url` in the user's default browser (best-effort; failure is ignored
/// because the URL and a QR are also shown in the prompt).
fn open_url_in_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Builds the option preview shown beside an authorization prompt: the full URL
/// (the single-line prompt title cannot render a long URL untruncated) plus the
/// saved QR image path.
pub(super) fn build_auth_preview(url: &str, qr_path: Option<&str>) -> String {
    let mut preview = format!("Authorization URL:\n{url}");
    if let Some(path) = qr_path {
        preview.push_str(&format!("\n\nOr scan the QR image:\n{path}"));
    }
    preview
}

/// Extracts the first whitespace-delimited `http(s)` URL from `line`, trimming
/// trailing punctuation. Used to scrape the verification URL `config init --new`
/// prints to stderr.
pub(super) fn first_http_url(line: &str) -> Option<String> {
    let start = line.find("http")?;
    let url = line[start..]
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches(['"', ')', ']', ',', '.'])
        .to_string();
    url.starts_with("http").then_some(url)
}

fn safe_lark_log_url(url: &str) -> String {
    let trimmed = url.trim_end_matches(['"', '\'', ')', ']', ',', '.']);
    let suffix = &url[trimmed.len()..];
    let base = trimmed.split('?').next().unwrap_or(trimmed);
    if trimmed.contains('?') {
        format!("{base}?<redacted>{suffix}")
    } else {
        format!("{base}{suffix}")
    }
}

fn sanitize_lark_cli_log_line(line: &str) -> String {
    line.split_whitespace()
        .map(|token| {
            if let Some(start) = token.find("http") {
                let (prefix, url) = token.split_at(start);
                format!("{prefix}{}", safe_lark_log_url(url))
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn push_lark_cli_log_line(lines: &std::sync::Arc<std::sync::Mutex<Vec<String>>>, line: String) {
    let mut lines = lines.lock().unwrap();
    lines.push(line);
    if lines.len() > 20 {
        lines.remove(0);
    }
}

fn lark_cli_log_tail(lines: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> String {
    let lines = lines.lock().unwrap();
    if lines.is_empty() {
        "no stderr lines captured".to_string()
    } else {
        lines.join(" | ")
    }
}

/// Drives the `lark-cli` device-flow login: show the verification URL/QR, wait
/// for the user to authorize in a browser, then complete with the device code.
fn ensure_lark_cli_logged_in(
    state: &mut AppState,
    resources: &LoadedResources,
    bin: &str,
) -> Result<()> {
    let started = lark_cli_output_timed(
        bin,
        &["auth", "login", "--no-wait", "--json", "--domain", "im"],
        std::time::Duration::from_secs(30),
    )?;
    if !started.status.success() {
        let stderr = String::from_utf8_lossy(&started.stderr);
        bail!("`{bin} auth login` could not start: {}", stderr.trim());
    }
    let payload: Value = serde_json::from_slice(&started.stdout)
        .context("parse `lark-cli auth login --no-wait --json` output")?;
    let url = payload
        .get("verification_url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("lark-cli did not return a verification_url"))?;
    let device_code = payload
        .get("device_code")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("lark-cli did not return a device_code"))?;

    // Open the URL directly and put the full URL + QR path in the option preview:
    // the single-line prompt title cannot render newlines or a long URL cleanly.
    open_url_in_browser(url);
    let qr_path = lark_cli_png_qr(bin, url);
    let preview = build_auth_preview(url, qr_path.as_deref());
    let choice = ask_choice_with_preview(
        state,
        resources,
        "Authorize",
        "Opened the Lark authorization page in your browser — authorize there, then choose Done. (URL & QR in the preview.)",
        &[
            (
                "Done",
                "I authorized Lark in the browser.",
                Some(preview.as_str()),
            ),
            ("Cancel", "Stop the Lark login.", None),
        ],
    )?;
    if choice != "Done" {
        bail!("Lark login cancelled");
    }

    let completed = lark_cli_output_timed(
        bin,
        &["auth", "login", "--device-code", device_code],
        std::time::Duration::from_secs(180),
    )?;
    if !completed.status.success() {
        let stderr = String::from_utf8_lossy(&completed.stderr);
        bail!(
            "Lark login did not complete: {}. Finish the browser authorization, then retry /connect.",
            stderr.trim()
        );
    }
    if !lark_cli_identity_ready(bin, "user") {
        bail!(
            "Lark login finished but `{bin} auth status` still reports no user identity; retry /connect."
        );
    }
    Ok(())
}

/// Sets up the app credentials backing lark-cli: either creates a new Feishu or
/// Lark app in the browser (`config init --new`) or configures an existing app
/// id + secret. lark-cli stores credentials in the OS keychain — Puffer never
/// persists them.
fn ensure_lark_cli_app_configured(
    state: &mut AppState,
    resources: &LoadedResources,
    bin: &str,
    expected_identity: &str,
) -> Result<()> {
    let brand = ask_lark_brand(state, resources)?;
    let product = lark_brand_product_name(&brand);
    let method_question = format!(
        "Puffer needs a {product} app configuration before connection can start. How should it be set up?"
    );
    let create_description =
        format!("Create a new {product} app in the browser (config init --new).");
    let method = ask_choice(
        state,
        resources,
        "App setup",
        &method_question,
        &[
            ("Create new app", &create_description),
            (
                "Use existing app",
                "Enter an existing app id and secret (no browser).",
            ),
        ],
    )?;
    if method == "Create new app" {
        lark_cli_config_init_new(state, resources, bin, &brand)?;
    } else {
        lark_cli_config_init_existing(state, resources, bin, &brand)?;
    }
    if expected_identity == "bot" && !lark_cli_identity_ready(bin, "bot") {
        bail!(
            "Lark app configured but `{bin} auth status` still reports no bot identity; \
             retry `/connect lark-bot`."
        );
    }
    Ok(())
}

fn ask_lark_brand(state: &mut AppState, resources: &LoadedResources) -> Result<String> {
    let answer = ask_choice(
        state,
        resources,
        "Brand",
        "Which product should this app use?",
        &[("Feishu", "China."), ("Lark", "International.")],
    )?;
    Ok(lark_brand_cli_value(&answer))
}

/// Normalizes a displayed Lark product answer to the CLI brand value.
pub(super) fn lark_brand_cli_value(answer: &str) -> String {
    match answer.trim().to_ascii_lowercase().as_str() {
        "lark" => "lark".to_string(),
        _ => "feishu".to_string(),
    }
}

fn lark_brand_product_name(brand: &str) -> &'static str {
    if brand == "lark" {
        "Lark"
    } else {
        "Feishu"
    }
}

/// Configures an existing Feishu/Lark app by prompting for its id, brand, and
/// secret; the secret is piped via stdin so it stays out of the process list.
fn lark_cli_config_init_existing(
    state: &mut AppState,
    resources: &LoadedResources,
    bin: &str,
    brand: &str,
) -> Result<()> {
    let app_id = ask_input(
        state,
        resources,
        "App ID",
        "What Feishu/Lark app id (cli_...) should Puffer use?",
    )?;
    let app_secret = ask_input(
        state,
        resources,
        "App secret",
        "What is the Feishu/Lark app secret? lark-cli stores it in your OS keychain; Puffer does not keep it.",
    )?;
    let output = lark_cli_config_init(bin, app_id.trim(), brand, app_secret.trim())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`{bin} config init` failed: {}", stderr.trim());
    }
    Ok(())
}

/// Builds the `lark-cli config init --new` argument list for a brand.
pub(super) fn lark_cli_config_init_new_args(brand: &str) -> [&str; 5] {
    ["config", "init", "--new", "--brand", brand]
}

/// Creates a new Feishu/Lark app via `config init --new`, which prints a
/// verification URL to stderr and blocks until the user finishes app creation in
/// the browser. Opens the URL, shows it (and a QR) in the prompt, and waits for
/// completion.
fn lark_cli_config_init_new(
    state: &mut AppState,
    resources: &LoadedResources,
    bin: &str,
    brand: &str,
) -> Result<()> {
    use std::io::{BufRead, BufReader};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant};

    let args = lark_cli_config_init_new_args(brand);
    eprintln!(
        "[lark-connect][config-init-new] start brand={brand} command=`{bin} {}`",
        args.join(" ")
    );
    let flow_start = Instant::now();
    let mut command = lark_cli_command(bin);
    let mut child = command
        .args(args)
        .stdin(std::process::Stdio::null())
        // stdout is unused (URL + QR go to stderr); null it so a chatty stdout
        // can't fill the pipe and deadlock the command while it waits.
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn `{bin} config init --new --brand {brand}`"))?;
    let child_id = child.id();
    eprintln!("[lark-connect][config-init-new] spawned pid={child_id} brand={brand}");
    // The URL (and a QR) are printed to stderr before it blocks; read stderr on a
    // thread so the pipe never fills while the command waits for the browser.
    let stderr = child
        .stderr
        .take()
        .context("open config init --new stderr")?;
    let (tx, rx) = mpsc::channel();
    let stderr_lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let stderr_lines_thread = stderr_lines.clone();
    std::thread::spawn(move || {
        let mut sent = false;
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let sanitized = sanitize_lark_cli_log_line(&line);
            push_lark_cli_log_line(&stderr_lines_thread, sanitized);
            if !sent {
                if let Some(url) = first_http_url(&line) {
                    let _ = tx.send(url);
                    sent = true;
                }
            }
        }
    });
    let url = match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(url) => url,
        Err(_) => {
            terminate_lark_cli_child(&mut child);
            let tail = lark_cli_log_tail(&stderr_lines);
            eprintln!(
                "[lark-connect][config-init-new] verification_url_timeout pid={child_id} elapsed_ms={} stderr_tail={tail}",
                flow_start.elapsed().as_millis()
            );
            bail!("`{bin} config init --new` did not print a verification URL; stderr: {tail}");
        }
    };
    eprintln!(
        "[lark-connect][config-init-new] verification_url_detected pid={child_id} elapsed_ms={} url={}",
        flow_start.elapsed().as_millis(),
        safe_lark_log_url(&url)
    );

    open_url_in_browser(&url);
    let qr_path = lark_cli_png_qr(bin, &url);
    let preview = build_auth_preview(&url, qr_path.as_deref());
    let choice = ask_choice_with_preview(
        state,
        resources,
        "Create app",
        "Opened the app-creation page in your browser — finish creating the app there, then choose Done. (URL & QR in the preview.)",
        &[
            (
                "Done",
                "I finished creating the app in the browser.",
                Some(preview.as_str()),
            ),
            ("Cancel", "Stop creating the Lark app.", None),
        ],
    )?;
    eprintln!(
        "[lark-connect][config-init-new] user_choice pid={child_id} choice={choice:?} elapsed_ms={}",
        flow_start.elapsed().as_millis()
    );
    if choice != "Done" {
        terminate_lark_cli_child(&mut child);
        bail!("App creation cancelled");
    }

    // The command self-completes once the browser flow finishes; wait for it.
    let start = Instant::now();
    let mut last_wait_log = Instant::now();
    eprintln!(
        "[lark-connect][config-init-new] waiting_for_registration pid={child_id} brand={brand}"
    );
    let status = loop {
        if let Some(status) = child.try_wait().context("wait for config init --new")? {
            break status;
        }
        if start.elapsed() >= Duration::from_secs(180) {
            terminate_lark_cli_child(&mut child);
            let tail = lark_cli_log_tail(&stderr_lines);
            eprintln!(
                "[lark-connect][config-init-new] registration_timeout pid={child_id} elapsed_ms={} stderr_tail={tail}",
                flow_start.elapsed().as_millis()
            );
            bail!(
                "`{bin} config init --new` did not finish. Close this dialog and click Connect again to start a fresh Feishu/Lark app-registration URL. stderr: {tail}"
            );
        }
        if last_wait_log.elapsed() >= Duration::from_secs(15) {
            last_wait_log = Instant::now();
            eprintln!(
                "[lark-connect][config-init-new] still_waiting pid={child_id} elapsed_ms={}",
                flow_start.elapsed().as_millis()
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    if !status.success() {
        let tail = lark_cli_log_tail(&stderr_lines);
        eprintln!(
            "[lark-connect][config-init-new] exit_failure pid={child_id} status={status} elapsed_ms={} stderr_tail={tail}",
            flow_start.elapsed().as_millis()
        );
        bail!("`{bin} config init --new` failed ({status}); stderr: {tail}");
    }
    eprintln!(
        "[lark-connect][config-init-new] exit_success pid={child_id} elapsed_ms={}",
        flow_start.elapsed().as_millis()
    );
    Ok(())
}

/// Runs `lark-cli config init`, feeding the app secret over stdin.
fn lark_cli_config_init(
    bin: &str,
    app_id: &str,
    brand: &str,
    app_secret: &str,
) -> Result<std::process::Output> {
    use std::io::Write as _;
    let mut child = std::process::Command::new(bin)
        .args([
            "config",
            "init",
            "--app-id",
            app_id,
            "--app-secret-stdin",
            "--brand",
            brand,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn `{bin} config init`"))?;
    // Write the secret, then always reap the child — even if the write fails
    // (e.g. lark-cli rejected the app id and exited before reading stdin, so the
    // pipe broke), so we never leak a zombie. The real outcome is its exit status.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(app_secret.as_bytes());
    }
    child
        .wait_with_output()
        .context("collect config init output")
}

/// Returns whether a helper program (e.g. `npm`, `go`) is on `PATH`.
fn program_available(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The official larksuite/cli one-liner installer args (needs Node.js for `npx`).
const LARK_CLI_INSTALL_ARGS: &[&str] = &["--yes", "@larksuite/cli@latest", "install"];

/// Manual install instructions shown when auto-install is declined or impossible.
fn lark_cli_install_instructions(bin: &str) -> String {
    format!(
        "Install the official larksuite/cli (needs Node.js for `npx`), then retry \
         `/connect lark-login` or `/connect lark-bot`:\n  npx @larksuite/cli@latest install\n\
         Then run `{bin} auth login` to sign in. Set LARK_CLI_BIN to point at a \
         non-PATH binary."
    )
}

/// Asks for consent then runs the official installer; bails (with guidance) if `npx` is missing or declined.
fn ensure_lark_cli_installed(
    state: &mut AppState,
    resources: &LoadedResources,
    bin: &str,
) -> Result<()> {
    let choice = ask_choice(
        state,
        resources,
        "Install",
        &format!("`{bin}` is not installed. Install the official larksuite/cli now?"),
        &[
            (
                "Install now",
                "Run `npx @larksuite/cli@latest install` (requires Node.js).",
            ),
            (
                "Show instructions",
                "Do not install; print the manual install command.",
            ),
        ],
    );
    // Headless (no interactive prompt): show guidance instead of an opaque error.
    let choice = match choice {
        Ok(choice) => choice,
        Err(_) => bail!(
            "`{bin}` is not installed.\n{}",
            lark_cli_install_instructions(bin)
        ),
    };
    if choice != "Install now" {
        bail!("{}", lark_cli_install_instructions(bin));
    }

    if !program_available("npx") {
        bail!(
            "`npx` (Node.js) is not available. Install Node.js first, then retry \
             `/connect lark-login` or `/connect lark-bot`.\n{}",
            lark_cli_install_instructions(bin)
        );
    }
    let output = std::process::Command::new("npx")
        .args(LARK_CLI_INSTALL_ARGS)
        .output()
        .with_context(|| format!("run `npx {}`", LARK_CLI_INSTALL_ARGS.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "`npx {}` failed to install lark-cli: {}\n{}",
            LARK_CLI_INSTALL_ARGS.join(" "),
            stderr.trim(),
            lark_cli_install_instructions(bin)
        );
    }
    if !lark_cli_available(bin) {
        bail!(
            "Installer ran, but `{bin}` is still not runnable. It may not be on \
             PATH — set LARK_CLI_BIN to its path, then retry /connect.\n{}",
            lark_cli_install_instructions(bin)
        );
    }
    Ok(())
}
