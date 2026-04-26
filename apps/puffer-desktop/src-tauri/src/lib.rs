mod auth_data;
mod daemon_launcher;
mod dtos;
mod remote_client;
mod repo_actions;
mod session_data;
mod settings_data;
mod turn;

use crate::daemon_launcher::{DaemonHandshake, DaemonLauncher};
use crate::dtos::{
    ExternalCredentialDto, FolderGroupDto, RemoteOperationDto, RepoActionResultDto, RepoStatusDto,
    SessionDetailDto, SettingsSnapshotDto,
};
use crate::turn::TurnRegistry;
use anyhow::Result;
use std::sync::Arc;
use tauri::{Builder, Manager, State};

#[tauri::command]
fn list_grouped_sessions(
    remote_target: Option<String>,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
) -> Result<Vec<FolderGroupDto>, String> {
    if let Some(target) = remote_target.filter(|value| !value.trim().is_empty()) {
        return remote_client::run_remote_json(
            &target,
            remote_cwd.as_deref(),
            remote_password.as_deref(),
            &[String::from("session-groups")],
        )
        .map_err(|error| error.to_string());
    }
    session_data::list_grouped_sessions().map_err(|error| error.to_string())
}

#[tauri::command]
fn load_session_detail(
    session_id: String,
    remote_target: Option<String>,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
) -> Result<SessionDetailDto, String> {
    if let Some(target) = remote_target.filter(|value| !value.trim().is_empty()) {
        return remote_client::run_remote_json(
            &target,
            remote_cwd.as_deref(),
            remote_password.as_deref(),
            &[String::from("session-detail"), session_id],
        )
        .map_err(|error| error.to_string());
    }
    session_data::load_session_detail(&session_id).map_err(|error| error.to_string())
}

#[tauri::command]
fn refresh_repo_status(
    session_id: String,
    remote_target: Option<String>,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
) -> Result<RepoStatusDto, String> {
    if let Some(target) = remote_target.filter(|value| !value.trim().is_empty()) {
        return remote_client::run_remote_json(
            &target,
            remote_cwd.as_deref(),
            remote_password.as_deref(),
            &[String::from("repo-status"), session_id],
        )
        .map_err(|error| error.to_string());
    }
    let cwd = session_data::load_session_cwd(&session_id).map_err(|error| error.to_string())?;
    Ok(repo_actions::repo_status(&session_id, &cwd))
}

#[tauri::command]
fn create_pull_request(
    session_id: String,
    title: Option<String>,
    body: Option<String>,
    remote_target: Option<String>,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
) -> Result<RepoActionResultDto, String> {
    if let Some(target) = remote_target.filter(|value| !value.trim().is_empty()) {
        let mut args = vec![String::from("create-pull-request"), session_id];
        if let Some(title) = title.as_deref() {
            args.push(String::from("--title"));
            args.push(title.to_string());
        }
        if let Some(body) = body.as_deref() {
            args.push(String::from("--body"));
            args.push(body.to_string());
        }
        return remote_client::run_remote_json(
            &target,
            remote_cwd.as_deref(),
            remote_password.as_deref(),
            &args,
        )
        .map_err(|error| error.to_string());
    }
    let cwd = session_data::load_session_cwd(&session_id).map_err(|error| error.to_string())?;
    Ok(repo_actions::create_pull_request(
        &session_id,
        &cwd,
        title,
        body,
    ))
}

#[tauri::command]
fn merge_pull_request(
    session_id: String,
    pull_request_number: Option<u64>,
    merge_method: Option<String>,
    remote_target: Option<String>,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
) -> Result<RepoActionResultDto, String> {
    if let Some(target) = remote_target.filter(|value| !value.trim().is_empty()) {
        let mut args = vec![String::from("merge-pull-request"), session_id];
        if let Some(number) = pull_request_number {
            args.push(String::from("--pull-request-number"));
            args.push(number.to_string());
        }
        if let Some(method) = merge_method.as_deref() {
            args.push(String::from("--merge-method"));
            args.push(method.to_string());
        }
        return remote_client::run_remote_json(
            &target,
            remote_cwd.as_deref(),
            remote_password.as_deref(),
            &args,
        )
        .map_err(|error| error.to_string());
    }
    let cwd = session_data::load_session_cwd(&session_id).map_err(|error| error.to_string())?;
    Ok(repo_actions::merge_pull_request(
        &session_id,
        &cwd,
        pull_request_number,
        merge_method,
    ))
}

#[tauri::command]
fn load_settings_snapshot(
    remote_target: Option<String>,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
) -> Result<SettingsSnapshotDto, String> {
    if let Some(target) = remote_target.filter(|value| !value.trim().is_empty()) {
        return remote_client::run_remote_json(
            &target,
            remote_cwd.as_deref(),
            remote_password.as_deref(),
            &[String::from("settings-snapshot")],
        )
        .map_err(|error| error.to_string());
    }
    settings_data::load_settings_snapshot().map_err(|error| error.to_string())
}

#[tauri::command]
fn login_with_oauth(
    provider_id: String,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
    remote_target: Option<String>,
) -> Result<SettingsSnapshotDto, String> {
    if let Some(target) = remote_target.filter(|value| !value.trim().is_empty()) {
        let snapshot: SettingsSnapshotDto = remote_client::run_remote_json(
            &target,
            remote_cwd.as_deref(),
            remote_password.as_deref(),
            &[String::from("settings-snapshot")],
        )
        .map_err(|error| error.to_string())?;
        let provider = snapshot
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| format!("unknown remote provider `{provider_id}`"))?;
        if !provider.auth_modes.iter().any(|mode| mode == "oauth") {
            return Err(format!(
                "remote provider `{provider_id}` does not support OAuth"
            ));
        }
        let credential = auth_data::acquire_oauth_credential(&provider_id, &provider.default_api)
            .map_err(|error| error.to_string())?;
        let credential_json =
            serde_json::to_string(&credential).map_err(|error| error.to_string())?;
        return remote_client::run_remote_json(
            &target,
            remote_cwd.as_deref(),
            remote_password.as_deref(),
            &[
                String::from("store-credential"),
                provider_id,
                String::from("--credential-json"),
                credential_json,
            ],
        )
        .map_err(|error| error.to_string());
    }
    auth_data::login_with_oauth(&provider_id).map_err(|error| error.to_string())
}

#[tauri::command]
fn login_with_api_key(
    provider_id: String,
    api_key: String,
    remote_target: Option<String>,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
) -> Result<SettingsSnapshotDto, String> {
    if let Some(target) = remote_target.filter(|value| !value.trim().is_empty()) {
        return remote_client::run_remote_json(
            &target,
            remote_cwd.as_deref(),
            remote_password.as_deref(),
            &[
                String::from("login-api-key"),
                provider_id,
                String::from("--api-key"),
                api_key,
            ],
        )
        .map_err(|error| error.to_string());
    }
    auth_data::login_with_api_key(&provider_id, &api_key).map_err(|error| error.to_string())
}

#[tauri::command]
fn logout_provider(
    provider_id: String,
    remote_target: Option<String>,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
) -> Result<SettingsSnapshotDto, String> {
    if let Some(target) = remote_target.filter(|value| !value.trim().is_empty()) {
        return remote_client::run_remote_json(
            &target,
            remote_cwd.as_deref(),
            remote_password.as_deref(),
            &[String::from("logout"), provider_id],
        )
        .map_err(|error| error.to_string());
    }
    auth_data::logout_provider(&provider_id).map_err(|error| error.to_string())
}

#[tauri::command]
fn list_external_credentials() -> Result<Vec<ExternalCredentialDto>, String> {
    auth_data::list_external_credentials().map_err(|error| error.to_string())
}

#[tauri::command]
fn import_external_credential(
    provider_id: String,
    source: String,
) -> Result<SettingsSnapshotDto, String> {
    auth_data::import_external_credential(&provider_id, &source).map_err(|error| error.to_string())
}

#[tauri::command]
fn run_remote_bash(
    remote_target: String,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
    command: String,
) -> Result<RemoteOperationDto, String> {
    remote_client::run_remote_shell(
        &remote_target,
        remote_cwd.as_deref(),
        remote_password.as_deref(),
        &command,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn read_remote_file(
    remote_target: String,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
    path: String,
) -> Result<RemoteOperationDto, String> {
    let command = format!("cat {}", remote_client::shell_quote(&path));
    remote_client::run_remote_shell(
        &remote_target,
        remote_cwd.as_deref(),
        remote_password.as_deref(),
        &command,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn write_remote_file(
    remote_target: String,
    remote_cwd: Option<String>,
    remote_password: Option<String>,
    path: String,
    contents_base64: String,
) -> Result<RemoteOperationDto, String> {
    let command = format!(
        "mkdir -p $(dirname {path}) && printf %s {contents} | base64 -d > {path}",
        path = remote_client::shell_quote(&path),
        contents = remote_client::shell_quote(&contents_base64)
    );
    remote_client::run_remote_shell(
        &remote_target,
        remote_cwd.as_deref(),
        remote_password.as_deref(),
        &command,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn run_agent_turn(
    app: tauri::AppHandle,
    registry: State<'_, Arc<TurnRegistry>>,
    session_id: String,
    message: String,
) -> Result<String, String> {
    turn::run_agent_turn(app, registry.inner().clone(), session_id, message)
}

#[tauri::command]
fn resolve_permission(
    registry: State<'_, Arc<TurnRegistry>>,
    turn_id: String,
    request_id: String,
    action: String,
) -> Result<(), String> {
    turn::resolve_permission(registry.inner().clone(), turn_id, request_id, action)
}

#[tauri::command]
fn cancel_turn(
    registry: State<'_, Arc<TurnRegistry>>,
    turn_id: String,
) -> Result<(), String> {
    turn::cancel_turn(registry.inner().clone(), turn_id)
}

/// Ensures the local `puffer daemon` subprocess is running and returns
/// its WebSocket URL + auth token so the frontend can connect directly.
/// The daemon dies with the Tauri parent process.
#[tauri::command]
fn start_local_daemon(
    launcher: State<'_, Arc<DaemonLauncher>>,
) -> Result<DaemonHandshake, String> {
    launcher
        .inner()
        .ensure_started()
        .map_err(|e| format!("{e:#}"))
}

/// Tears down the current local `puffer daemon` subprocess and spawns a
/// new one rooted at `cwd`. Used by the WorkspacePicker's
/// "Switch local workspace" action — after this returns the caller must
/// call `switchDaemonClient(handshake)` on the frontend so the shared
/// DaemonClient connects to the new daemon instead of the old (now
/// defunct) one.
#[tauri::command]
fn restart_local_daemon(
    launcher: State<'_, Arc<DaemonLauncher>>,
    cwd: String,
) -> Result<DaemonHandshake, String> {
    let path = std::path::PathBuf::from(&cwd);
    if !path.exists() {
        return Err(format!("workspace path does not exist: {cwd}"));
    }
    if !path.is_dir() {
        return Err(format!("workspace path is not a directory: {cwd}"));
    }
    launcher
        .inner()
        .restart_local(path)
        .map_err(|e| format!("{e:#}"))
}

/// Starts a `puffer daemon` on a remote host over SSH, forwards its
/// WebSocket port to a local port, and returns a handshake pointing at the
/// local end. The frontend uses the returned URL with its normal
/// DaemonClient so tunneled remotes look local.
///
/// `ssh_target` is the usual `[user@]host` form. Optional `remote_binary`
/// overrides the remote `puffer` path (defaults to `puffer` on $PATH).
/// Optional `remote_workspace` makes the daemon `cd` before starting so
/// sessions live under that path's `.puffer/`.
#[tauri::command]
fn start_ssh_daemon(
    launcher: State<'_, Arc<DaemonLauncher>>,
    ssh_target: String,
    remote_binary: Option<String>,
    remote_workspace: Option<String>,
) -> Result<DaemonHandshake, String> {
    launcher
        .inner()
        .start_ssh(
            &ssh_target,
            remote_binary.as_deref(),
            remote_workspace.as_deref(),
        )
        .map_err(|e| format!("{e:#}"))
}

/// Runs the Puffer Desktop Tauri host.
pub fn run() {
    // The in-process Tauri commands (e.g. `load_settings_snapshot`) discover
    // resources relative to `current_dir()`, which on a packaged app is wherever
    // the OS launched the binary from — almost never the puffer repo root, so
    // the LoginView ends up with an empty provider catalog. Point the
    // resource loader at the bundled `resources/` dir we ship next to the
    // binary if one is discoverable. Honors a pre-set env var so users can
    // override the catalog location.
    if std::env::var_os("PUFFER_BUILTIN_RESOURCES_DIR").is_none() {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(resources_dir) = daemon_launcher::resolve_builtin_resources_dir(&exe) {
                std::env::set_var("PUFFER_BUILTIN_RESOURCES_DIR", resources_dir);
            }
        }
    }
    Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(Arc::new(TurnRegistry::new()));
            app.manage(Arc::new(DaemonLauncher::new()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_grouped_sessions,
            load_session_detail,
            refresh_repo_status,
            create_pull_request,
            merge_pull_request,
            load_settings_snapshot,
            login_with_oauth,
            login_with_api_key,
            logout_provider,
            list_external_credentials,
            import_external_credential,
            run_remote_bash,
            read_remote_file,
            write_remote_file,
            run_agent_turn,
            resolve_permission,
            cancel_turn,
            start_local_daemon,
            start_ssh_daemon,
            restart_local_daemon,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Puffer Desktop");
}
