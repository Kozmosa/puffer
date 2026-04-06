use puffer_config::{ensure_workspace_dirs, ConfigPaths};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[test]
fn top_level_help_shows_claude_style_public_surface() {
    let (_tempdir, workspace, puffer_home) = configured_workspace();
    let output = run_puffer(&workspace, &puffer_home, &["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for command in [
        "agents",
        "auth",
        "auto-mode",
        "doctor",
        "install",
        "mcp",
        "plugin",
        "setup-token",
        "update",
    ] {
        assert!(
            stdout.contains(command),
            "missing `{command}` in help:\n{stdout}"
        );
    }
    for hidden in [
        "anthropic-request-fixture",
        "providers",
        "resources",
        "sessions",
        "tool",
    ] {
        assert!(
            !stdout.contains(hidden),
            "unexpected `{hidden}` in help:\n{stdout}"
        );
    }
}

#[test]
fn auth_help_hides_internal_subcommands_and_logout_clears_tokens() {
    let (_tempdir, workspace, puffer_home) = configured_workspace();
    let help = run_puffer(&workspace, &puffer_home, &["auth", "--help"]);
    assert!(help.status.success());
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(help_text.contains("status"));
    assert!(help_text.contains("login"));
    assert!(help_text.contains("logout"));
    assert!(!help_text.contains("set-api-key"));
    assert!(!help_text.contains("oauth-start"));

    let setup = run_puffer(
        &workspace,
        &puffer_home,
        &["setup-token", "anthropic", "sk-test-token"],
    );
    assert!(setup.status.success(), "{setup:?}");

    let status = run_puffer(&workspace, &puffer_home, &["auth", "status", "--text"]);
    assert!(status.status.success(), "{status:?}");
    let status_text = String::from_utf8_lossy(&status.stdout);
    assert!(status_text.contains("anthropic (api_key)"), "{status_text}");

    let logout = run_puffer(&workspace, &puffer_home, &["auth", "logout", "anthropic"]);
    assert!(logout.status.success(), "{logout:?}");
    let logout_text = String::from_utf8_lossy(&logout.stdout);
    assert!(logout_text.contains("cleared credentials for anthropic"));

    let status = run_puffer(&workspace, &puffer_home, &["auth", "status", "--text"]);
    assert!(status.status.success(), "{status:?}");
    let status_text = String::from_utf8_lossy(&status.stdout);
    assert!(status_text.contains("No providers are authenticated."));
}

#[test]
fn resume_help_mentions_the_tui() {
    let (_tempdir, workspace, puffer_home) = configured_workspace();
    let output = run_puffer(&workspace, &puffer_home, &["resume", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Resume a stored session in the TUI"));
}

#[test]
fn mcp_round_trip_add_get_list_remove() {
    let (_tempdir, workspace, puffer_home) = configured_workspace();

    let add = run_puffer(
        &workspace,
        &puffer_home,
        &[
            "mcp",
            "add",
            "demo",
            "https://example.invalid/mcp",
            "--transport",
            "http",
        ],
    );
    assert!(add.status.success(), "{add:?}");

    let get = run_puffer(&workspace, &puffer_home, &["mcp", "get", "demo"]);
    assert!(get.status.success(), "{get:?}");
    let get_text = String::from_utf8_lossy(&get.stdout);
    assert!(get_text.contains("\"id\": \"demo\""));
    assert!(get_text.contains("\"transport\": \"http\""));

    let get_plugin = run_puffer(&workspace, &puffer_home, &["mcp", "get", "core:docs"]);
    assert!(get_plugin.status.success(), "{get_plugin:?}");
    let get_plugin_text = String::from_utf8_lossy(&get_plugin.stdout);
    assert!(get_plugin_text.contains("\"id\": \"core:docs\""));
    assert!(get_plugin_text.contains("\"plugin_id\": \"core\""));

    let list = run_puffer(&workspace, &puffer_home, &["mcp", "list"]);
    assert!(list.status.success(), "{list:?}");
    let list_text = String::from_utf8_lossy(&list.stdout);
    assert!(list_text.contains("demo [http]"));

    let remove = run_puffer(&workspace, &puffer_home, &["mcp", "remove", "demo"]);
    assert!(remove.status.success(), "{remove:?}");

    let list = run_puffer(&workspace, &puffer_home, &["mcp", "list"]);
    assert!(list.status.success(), "{list:?}");
    let list_text = String::from_utf8_lossy(&list.stdout);
    assert!(!list_text.contains("demo [http]"), "{list_text}");
}

#[test]
fn plugin_round_trip_install_disable_enable_validate_uninstall() {
    let (tempdir, workspace, puffer_home) = configured_workspace();
    let manifest_path = tempdir.path().join("demo-plugin.yaml");
    fs::write(
        &manifest_path,
        r#"id: demo
display_name: Demo Plugin
description: Demo plugin
commands:
  - name: demo
    description: Demo command
"#,
    )
    .expect("plugin manifest");

    let validate = run_puffer(
        &workspace,
        &puffer_home,
        &["plugin", "validate", manifest_path.to_str().unwrap()],
    );
    assert!(validate.status.success(), "{validate:?}");
    let validate_text = String::from_utf8_lossy(&validate.stdout);
    assert!(validate_text.contains("\"id\": \"demo\""));

    let install = run_puffer(
        &workspace,
        &puffer_home,
        &["plugin", "install", manifest_path.to_str().unwrap()],
    );
    assert!(install.status.success(), "{install:?}");

    let disable = run_puffer(&workspace, &puffer_home, &["plugin", "disable", "demo"]);
    assert!(disable.status.success(), "{disable:?}");

    let list = run_puffer(&workspace, &puffer_home, &["plugin", "list"]);
    assert!(list.status.success(), "{list:?}");
    let list_text = String::from_utf8_lossy(&list.stdout);
    assert!(list_text.contains("demo [disabled]"), "{list_text}");

    let enable = run_puffer(&workspace, &puffer_home, &["plugin", "enable", "demo"]);
    assert!(enable.status.success(), "{enable:?}");

    let list = run_puffer(&workspace, &puffer_home, &["plugin", "list"]);
    assert!(list.status.success(), "{list:?}");
    let list_text = String::from_utf8_lossy(&list.stdout);
    assert!(list_text.contains("demo [enabled]"), "{list_text}");

    let uninstall = run_puffer(&workspace, &puffer_home, &["plugin", "uninstall", "demo"]);
    assert!(uninstall.status.success(), "{uninstall:?}");

    let list = run_puffer(&workspace, &puffer_home, &["plugin", "list"]);
    assert!(list.status.success(), "{list:?}");
    let list_text = String::from_utf8_lossy(&list.stdout);
    assert!(!list_text.contains("demo [enabled]"), "{list_text}");
    assert!(!list_text.contains("demo [disabled]"), "{list_text}");
}

#[test]
fn builtin_plugin_disable_and_update_keep_disabled_state() {
    let (_tempdir, workspace, puffer_home) = configured_workspace();

    let disable = run_puffer(&workspace, &puffer_home, &["plugin", "disable", "example"]);
    assert!(disable.status.success(), "{disable:?}");

    let list = run_puffer(&workspace, &puffer_home, &["plugin", "list"]);
    assert!(list.status.success(), "{list:?}");
    let list_text = String::from_utf8_lossy(&list.stdout);
    assert!(list_text.contains("example [disabled]"), "{list_text}");

    let update = run_puffer(&workspace, &puffer_home, &["plugin", "update", "example"]);
    assert!(update.status.success(), "{update:?}");

    let list = run_puffer(&workspace, &puffer_home, &["plugin", "list"]);
    assert!(list.status.success(), "{list:?}");
    let list_text = String::from_utf8_lossy(&list.stdout);
    assert!(list_text.contains("example [disabled]"), "{list_text}");

    let enable = run_puffer(&workspace, &puffer_home, &["plugin", "enable", "example"]);
    assert!(enable.status.success(), "{enable:?}");

    let list = run_puffer(&workspace, &puffer_home, &["plugin", "list"]);
    assert!(list.status.success(), "{list:?}");
    let list_text = String::from_utf8_lossy(&list.stdout);
    assert!(list_text.contains("example [enabled]"), "{list_text}");
}

#[test]
fn install_and_update_commands_fail_when_not_implemented() {
    let (_tempdir, workspace, puffer_home) = configured_workspace();

    let install = run_puffer(&workspace, &puffer_home, &["install", "latest"]);
    assert!(!install.status.success(), "{install:?}");
    let install_text = String::from_utf8_lossy(&install.stderr);
    assert!(
        install_text.contains("does not ship a self-installer"),
        "{install_text}"
    );

    let update = run_puffer(&workspace, &puffer_home, &["update"]);
    assert!(!update.status.success(), "{update:?}");
    let update_text = String::from_utf8_lossy(&update.stderr);
    assert!(
        update_text.contains("does not include a self-updater"),
        "{update_text}"
    );
}

fn configured_workspace() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let workspace = tempdir.path().join("workspace");
    let puffer_home = tempdir.path().join("puffer-home");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&puffer_home).expect("puffer-home");
    let paths = ConfigPaths::discover(&workspace);
    ensure_workspace_dirs(&paths).expect("dirs");
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("cli crate parent")
        .parent()
        .expect("repo root");
    std::os::unix::fs::symlink(repo_root.join("resources"), workspace.join("resources"))
        .expect("resource symlink");
    (tempdir, workspace, puffer_home)
}

fn run_puffer(workspace: &Path, puffer_home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_puffer"))
        .args(args)
        .current_dir(workspace)
        .env("PUFFER_HOME", puffer_home)
        .output()
        .expect("puffer process")
}
