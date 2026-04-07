use super::emit_system;
use crate::AppState;
use anyhow::Result;
use puffer_session_store::SessionStore;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const NATIVE_CSIU_TERMINALS: [(&str, &str); 5] = [
    ("ghostty", "Ghostty"),
    ("kitty", "Kitty"),
    ("iTerm.app", "iTerm2"),
    ("WezTerm", "WezTerm"),
    ("WarpTerminal", "Warp"),
];

const VSCODE_KEYBINDING_SNIPPET: &str = "{\n  \"key\": \"shift+enter\",\n  \"command\": \"workbench.action.terminal.sendSequence\",\n  \"args\": { \"text\": \"\\\\u001b\\\\r\" },\n  \"when\": \"terminalFocus\"\n}";
const ALACRITTY_KEYBINDING_SNIPPET: &str =
    "[[keyboard.bindings]]\nkey = \"Return\"\nmods = \"Shift\"\nchars = \"\\\\u001B\\\\r\"";
const ZED_KEYBINDING_SNIPPET: &str = "{\n  \"context\": \"Terminal\",\n  \"bindings\": {\n    \"shift-enter\": [\"terminal::SendText\", \"\\\\u001b\\\\r\"]\n  }\n}";

#[derive(Debug, Clone, PartialEq, Eq)]
enum DetectedTerminal {
    Native(&'static str),
    Supported(SupportedTerminal),
    Unsupported(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportedTerminal {
    AppleTerminal,
    VsCode,
    Cursor,
    Windsurf,
    Alacritty,
    Zed,
}

/// Returns the Claude-style `/terminal-setup` description for the current terminal.
pub(crate) fn terminal_setup_command_description() -> &'static str {
    description_for_detected(&detect_terminal())
}

/// Returns whether `/terminal-setup` should be hidden from slash-command UI surfaces.
pub(crate) fn should_hide_terminal_setup_command() -> bool {
    hidden_for_detected(&detect_terminal())
}

/// Handles `/terminal-setup` by installing supported keybindings or falling
/// back to Claude-style terminal guidance.
pub(crate) fn handle_terminal_setup_command(
    state: &mut AppState,
    session_store: &SessionStore,
) -> Result<()> {
    emit_system(state, session_store, terminal_setup_result())
}

fn render_supported_terminal_guidance(terminal: SupportedTerminal) -> String {
    match terminal {
        SupportedTerminal::AppleTerminal => render_apple_terminal_guidance(),
        SupportedTerminal::VsCode => render_vscode_guidance("VSCode", "Code"),
        SupportedTerminal::Cursor => render_vscode_guidance("Cursor", "Cursor"),
        SupportedTerminal::Windsurf => render_vscode_guidance("Windsurf", "Windsurf"),
        SupportedTerminal::Alacritty => render_alacritty_guidance(),
        SupportedTerminal::Zed => render_zed_guidance(),
    }
}

fn description_for_detected(detected: &DetectedTerminal) -> &'static str {
    match detected {
        DetectedTerminal::Supported(SupportedTerminal::AppleTerminal) => {
            "Enable Option+Enter key binding for newlines and visual bell"
        }
        _ => "Install Shift+Enter key binding for newlines",
    }
}

fn hidden_for_detected(detected: &DetectedTerminal) -> bool {
    matches!(detected, DetectedTerminal::Native(_))
}

fn terminal_setup_result() -> String {
    match detect_terminal() {
        DetectedTerminal::Native(display_name) => format!(
            "Shift+Enter is natively supported in {display_name}.\n\nNo configuration needed. Just use Shift+Enter to add newlines."
        ),
        DetectedTerminal::Supported(terminal) => install_supported_terminal(terminal)
            .unwrap_or_else(|error| format!("{error}\n\n{}", render_supported_terminal_guidance(terminal))),
        DetectedTerminal::Unsupported(terminal_name) => {
            render_unsupported_terminal_guidance(&terminal_name)
        }
    }
}

fn install_supported_terminal(terminal: SupportedTerminal) -> Result<String, String> {
    match terminal {
        SupportedTerminal::AppleTerminal => install_apple_terminal_setup(),
        SupportedTerminal::VsCode => install_vscode_keybinding("VSCode", "Code"),
        SupportedTerminal::Cursor => install_vscode_keybinding("Cursor", "Cursor"),
        SupportedTerminal::Windsurf => install_vscode_keybinding("Windsurf", "Windsurf"),
        SupportedTerminal::Alacritty => install_alacritty_keybinding(),
        SupportedTerminal::Zed => install_zed_keybinding(),
    }
}

fn render_apple_terminal_guidance() -> String {
    format!(
        "Configure Terminal.app settings:\n- Open Terminal.app and go to Settings -> Profiles -> Keyboard.\n- Enable \"Use Option as Meta key\" for your default and startup profiles.\n- Switch to a visual bell (or disable the audio bell) for those profiles.\n- Restart Terminal.app after changing the settings.\n\nOption+Enter will then enter a newline.\nPreferences file: {}",
        apple_terminal_preferences_path().display()
    )
}

fn render_vscode_guidance(editor_name: &str, editor_dir: &str) -> String {
    if is_vscode_remote_ssh() {
        return format!(
            "Cannot install keybindings from a remote {editor_name} session.\n\n{editor_name} keybindings must be installed on your local machine, not the remote server.\n\nTo install the Shift+Enter keybinding:\n1. Open {editor_name} on your local machine (not connected to remote).\n2. Open the Command Palette and run \"Preferences: Open Keyboard Shortcuts (JSON)\".\n3. Add this keybinding to the JSON array:\n\n{VSCODE_KEYBINDING_SNIPPET}"
        );
    }

    format!(
        "Install {editor_name} terminal Shift+Enter key binding.\n\nOpen {} and add this entry to the JSON array:\n\n{VSCODE_KEYBINDING_SNIPPET}\n\nIf the file already contains bindings, append the object instead of replacing the array.\nRestart {editor_name} if the binding does not take effect immediately.",
        vscode_keybindings_path(editor_dir).display()
    )
}

fn render_alacritty_guidance() -> String {
    format!(
        "Install Alacritty Shift+Enter key binding.\n\nAdd this block to {}:\n\n{ALACRITTY_KEYBINDING_SNIPPET}\n\nRestart Alacritty after updating the config.",
        alacritty_config_path().display()
    )
}

fn render_zed_guidance() -> String {
    format!(
        "Install Zed Shift+Enter key binding.\n\nOpen {} and add this entry to the JSON array:\n\n{ZED_KEYBINDING_SNIPPET}\n\nIf the file already contains bindings, append the object instead of replacing the array.",
        zed_keymap_path().display()
    )
}

fn install_vscode_keybinding(editor_name: &str, editor_dir: &str) -> Result<String, String> {
    install_vscode_keybinding_at(
        editor_name,
        &vscode_keybindings_path(editor_dir),
        is_vscode_remote_ssh(),
    )
}

fn install_vscode_keybinding_at(
    editor_name: &str,
    keybindings_path: &Path,
    remote_session: bool,
) -> Result<String, String> {
    if remote_session {
        return Err(format!(
            "Cannot install keybindings from a remote {editor_name} session.\n\n{editor_name} keybindings must be installed on your local machine, not the remote server."
        ));
    }

    if let Some(parent) = keybindings_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create {editor_name} keybindings directory: {error}\nPath: {}",
                parent.display()
            )
        })?;
    }

    let mut keybindings = match read_or_initialize_json_array(keybindings_path, true)? {
        Some(items) => items,
        None => {
            return Err(format!(
                "Could not safely edit the existing {editor_name} keybindings because the file is not strict JSON.\nPath: {}",
                keybindings_path.display()
            ));
        }
    };

    if keybindings.iter().any(|entry| {
        entry
            .get("key")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|key| key == "shift+enter")
    }) {
        return Ok(format!(
            "Found existing {editor_name} terminal Shift+Enter key binding. Remove it to continue.\nPath: {}",
            keybindings_path.display()
        ));
    }

    backup_existing_file(keybindings_path).map_err(|error| {
        format!(
            "Error backing up existing {editor_name} terminal keybindings: {error}\nPath: {}",
            keybindings_path.display()
        )
    })?;
    keybindings.push(serde_json::json!({
        "key": "shift+enter",
        "command": "workbench.action.terminal.sendSequence",
        "args": { "text": "\u{001b}\r" },
        "when": "terminalFocus",
    }));
    write_json_array(keybindings_path, &keybindings).map_err(|error| {
        format!(
            "Failed to install {editor_name} terminal Shift+Enter key binding: {error}\nPath: {}",
            keybindings_path.display()
        )
    })?;
    Ok(format!(
        "Installed {editor_name} terminal Shift+Enter key binding\nPath: {}",
        keybindings_path.display()
    ))
}

fn install_alacritty_keybinding() -> Result<String, String> {
    install_alacritty_keybinding_at(&alacritty_config_path())
}

fn install_alacritty_keybinding_at(config_path: &Path) -> Result<String, String> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Alacritty config directory: {error}\nPath: {}",
                parent.display()
            )
        })?;
    }

    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    if existing.contains("mods = \"Shift\"")
        && existing.contains("key = \"Return\"")
        && existing.contains("\\u001B\\r")
    {
        return Ok(format!(
            "Found existing Alacritty Shift+Enter key binding. Remove it to continue.\nPath: {}",
            config_path.display()
        ));
    }

    backup_existing_file(config_path).map_err(|error| {
        format!(
            "Error backing up existing Alacritty config: {error}\nPath: {}",
            config_path.display()
        )
    })?;
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push('\n');
    updated.push_str(ALACRITTY_KEYBINDING_SNIPPET);
    updated.push('\n');
    fs::write(&config_path, updated).map_err(|error| {
        format!(
            "Failed to install Alacritty Shift+Enter key binding: {error}\nPath: {}",
            config_path.display()
        )
    })?;
    Ok(format!(
        "Installed Alacritty Shift+Enter key binding\nYou may need to restart Alacritty for changes to take effect.\nPath: {}",
        config_path.display()
    ))
}

fn install_zed_keybinding() -> Result<String, String> {
    install_zed_keybinding_at(&zed_keymap_path())
}

fn install_zed_keybinding_at(keymap_path: &Path) -> Result<String, String> {
    if let Some(parent) = keymap_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Zed config directory: {error}\nPath: {}",
                parent.display()
            )
        })?;
    }

    let mut keymap = match read_or_initialize_json_array(keymap_path, true)? {
        Some(items) => items,
        None => {
            return Err(format!(
                "Could not safely edit the existing Zed keymap because the file is not strict JSON.\nPath: {}",
                keymap_path.display()
            ));
        }
    };

    if keymap.iter().any(|entry| {
        entry
            .get("bindings")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|bindings| bindings.contains_key("shift-enter"))
    }) {
        return Ok(format!(
            "Found existing Zed Shift+Enter key binding. Remove it to continue.\nPath: {}",
            keymap_path.display()
        ));
    }

    backup_existing_file(keymap_path).map_err(|error| {
        format!(
            "Error backing up existing Zed keymap: {error}\nPath: {}",
            keymap_path.display()
        )
    })?;
    keymap.push(serde_json::json!({
        "context": "Terminal",
        "bindings": {
            "shift-enter": ["terminal::SendText", "\u{001b}\r"],
        },
    }));
    write_json_array(keymap_path, &keymap).map_err(|error| {
        format!(
            "Failed to install Zed Shift+Enter key binding: {error}\nPath: {}",
            keymap_path.display()
        )
    })?;
    Ok(format!(
        "Installed Zed Shift+Enter key binding\nPath: {}",
        keymap_path.display()
    ))
}

fn install_apple_terminal_setup() -> Result<String, String> {
    if env::consts::OS != "macos" {
        return Err(
            "Terminal.app setup can only be installed on macOS.\nUse the manual steps below instead."
                .to_string(),
        );
    }

    let plist_path = apple_terminal_preferences_path();
    let backup = backup_existing_file(&plist_path).map_err(|error| {
        format!(
            "Failed to back up Terminal.app preferences: {error}\nPath: {}",
            plist_path.display()
        )
    })?;
    let default_profile = read_defaults_value("Default Window Settings")?;
    let startup_profile = read_defaults_value("Startup Window Settings")?;

    let mut profiles = vec![default_profile];
    if startup_profile != profiles[0] {
        profiles.push(startup_profile);
    }

    for profile in &profiles {
        if let Err(error) = update_terminal_profile(profile, "useOptionAsMetaKey", true)
            .and_then(|_| update_terminal_profile(profile, "Bell", false))
        {
            restore_backup(&plist_path, backup.as_deref());
            return Err(error);
        }
    }

    let _ = Command::new("killall").arg("cfprefsd").status();
    Ok(
        "Configured Terminal.app settings:\n- Enabled \"Use Option as Meta key\"\n- Switched to visual bell\nOption+Enter will now enter a newline.\nYou must restart Terminal.app for changes to take effect."
            .to_string(),
    )
}

fn render_unsupported_terminal_guidance(terminal_name: &str) -> String {
    let platform_terminals = match env::consts::OS {
        "macos" => "   • macOS: Apple Terminal\n",
        "windows" => "   • Windows: Windows Terminal\n",
        _ => "",
    };
    format!(
        "Terminal setup cannot be run from {terminal_name}.\n\nThis command configures a convenient Shift+Enter shortcut for multi-line prompts.\nNote: You can already use backslash (\\\\) + return to add newlines.\n\nTo set up the shortcut (optional):\n1. Exit tmux/screen temporarily\n2. Run /terminal-setup directly in one of these terminals:\n{platform_terminals}   • IDE: VSCode, Cursor, Windsurf, Zed\n   • Other: Alacritty\n3. Return to tmux/screen - settings will persist\n\nNote: iTerm2, WezTerm, Ghostty, Kitty, and Warp support Shift+Enter natively."
    )
}

fn detect_terminal() -> DetectedTerminal {
    if env_var_present("CURSOR_TRACE_ID") {
        return DetectedTerminal::Supported(SupportedTerminal::Cursor);
    }

    let askpass = env::var("VSCODE_GIT_ASKPASS_MAIN").unwrap_or_default();
    if askpass.contains("cursor") {
        return DetectedTerminal::Supported(SupportedTerminal::Cursor);
    }
    if askpass.contains("windsurf") {
        return DetectedTerminal::Supported(SupportedTerminal::Windsurf);
    }

    if env::var("TERM").ok().as_deref() == Some("xterm-ghostty") {
        return DetectedTerminal::Native("Ghostty");
    }
    if env::var("TERM")
        .ok()
        .as_deref()
        .is_some_and(|term| term.contains("kitty"))
    {
        return DetectedTerminal::Native("Kitty");
    }

    if let Some(program) = env::var("TERM_PROGRAM").ok() {
        if let Some(display_name) = native_terminal_display_name(program.as_str()) {
            return DetectedTerminal::Native(display_name);
        }
        if let Some(terminal) = supported_term_program(program.as_str()) {
            return DetectedTerminal::Supported(terminal);
        }
        return DetectedTerminal::Unsupported(human_terminal_name(program.as_str()));
    }

    if env_var_present("ALACRITTY_LOG") {
        return DetectedTerminal::Supported(SupportedTerminal::Alacritty);
    }
    if env_var_present("TMUX") {
        return DetectedTerminal::Unsupported("tmux".to_string());
    }
    if env_var_present("STY") {
        return DetectedTerminal::Unsupported("screen".to_string());
    }

    if env::var("TERM")
        .ok()
        .as_deref()
        .is_some_and(|term| term.contains("alacritty"))
    {
        return DetectedTerminal::Supported(SupportedTerminal::Alacritty);
    }

    DetectedTerminal::Unsupported("your current terminal".to_string())
}

fn env_var_present(name: &str) -> bool {
    env::var_os(name).is_some()
}

fn native_terminal_display_name(program: &str) -> Option<&'static str> {
    NATIVE_CSIU_TERMINALS
        .iter()
        .find(|(candidate, _)| *candidate == program)
        .map(|(_, display_name)| *display_name)
}

fn supported_term_program(program: &str) -> Option<SupportedTerminal> {
    match program {
        "Apple_Terminal" => Some(SupportedTerminal::AppleTerminal),
        "vscode" => Some(SupportedTerminal::VsCode),
        "cursor" => Some(SupportedTerminal::Cursor),
        "windsurf" => Some(SupportedTerminal::Windsurf),
        "alacritty" => Some(SupportedTerminal::Alacritty),
        "zed" => Some(SupportedTerminal::Zed),
        _ => None,
    }
}

fn human_terminal_name(program: &str) -> String {
    match program {
        "Apple_Terminal" => "Apple Terminal".to_string(),
        "iTerm.app" => "iTerm2".to_string(),
        "WezTerm" => "WezTerm".to_string(),
        "WarpTerminal" => "Warp".to_string(),
        "vscode" => "VSCode".to_string(),
        "cursor" => "Cursor".to_string(),
        "windsurf" => "Windsurf".to_string(),
        "alacritty" => "Alacritty".to_string(),
        "zed" => "Zed".to_string(),
        other => other.to_string(),
    }
}

fn is_vscode_remote_ssh() -> bool {
    let askpass = env::var("VSCODE_GIT_ASKPASS_MAIN").unwrap_or_default();
    let path = env::var("PATH").unwrap_or_default();
    askpass.contains(".vscode-server")
        || askpass.contains(".cursor-server")
        || askpass.contains(".windsurf-server")
        || path.contains(".vscode-server")
        || path.contains(".cursor-server")
        || path.contains(".windsurf-server")
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn apple_terminal_preferences_path() -> PathBuf {
    home_dir().join("Library/Preferences/com.apple.Terminal.plist")
}

fn vscode_keybindings_path(editor_dir: &str) -> PathBuf {
    match env::consts::OS {
        "windows" => env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join("AppData/Roaming"))
            .join(editor_dir)
            .join("User/keybindings.json"),
        "macos" => home_dir()
            .join("Library/Application Support")
            .join(editor_dir)
            .join("User/keybindings.json"),
        _ => home_dir()
            .join(".config")
            .join(editor_dir)
            .join("User/keybindings.json"),
    }
}

fn alacritty_config_path() -> PathBuf {
    if env::consts::OS == "windows" {
        return env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join("AppData/Roaming"))
            .join("alacritty/alacritty.toml");
    }
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config_home).join("alacritty/alacritty.toml");
    }
    home_dir().join(".config/alacritty/alacritty.toml")
}

fn zed_keymap_path() -> PathBuf {
    home_dir().join(".config/zed/keymap.json")
}

fn read_or_initialize_json_array(
    path: &Path,
    strict_existing: bool,
) -> Result<Option<Vec<serde_json::Value>>, String> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            if contents.trim().is_empty() {
                return Ok(Some(Vec::new()));
            }
            match serde_json::from_str::<Vec<serde_json::Value>>(&contents) {
                Ok(values) => Ok(Some(values)),
                Err(_) if strict_existing => Ok(None),
                Err(error) => Err(format!("Failed to parse {}: {error}", path.display())),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Some(Vec::new())),
        Err(error) => Err(format!("Failed to read {}: {error}", path.display())),
    }
}

fn write_json_array(path: &Path, values: &[serde_json::Value]) -> Result<(), String> {
    let mut serialized = serde_json::to_string_pretty(values)
        .map_err(|error| format!("failed to serialize JSON bindings: {error}"))?;
    serialized.push('\n');
    fs::write(path, serialized)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn backup_existing_file(path: &Path) -> Result<Option<PathBuf>, std::io::Error> {
    if !path.exists() {
        return Ok(None);
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "config".to_string());
    let backup = path.with_file_name(format!("{file_name}.{timestamp}.bak"));
    fs::copy(path, &backup)?;
    Ok(Some(backup))
}

fn restore_backup(path: &Path, backup: Option<&Path>) {
    let Some(backup) = backup else {
        return;
    };
    let _ = fs::copy(backup, path);
}

fn read_defaults_value(key: &str) -> Result<String, String> {
    let output = Command::new("defaults")
        .args(["read", "com.apple.Terminal", key])
        .output()
        .map_err(|error| format!("Failed to read Terminal.app setting `{key}`: {error}"))?;
    if !output.status.success() {
        return Err(format!("Failed to read Terminal.app setting `{key}`."));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        Err(format!("Terminal.app setting `{key}` is empty."))
    } else {
        Ok(value)
    }
}

fn update_terminal_profile(profile: &str, key: &str, value: bool) -> Result<(), String> {
    let plist = apple_terminal_preferences_path()
        .to_string_lossy()
        .to_string();
    let value_str = if value { "true" } else { "false" };
    let add_command = format!("Add :'Window Settings':'{profile}':{key} bool {value_str}");
    if plist_buddy(&plist, add_command.as_str()).is_ok() {
        return Ok(());
    }
    let set_command = format!("Set :'Window Settings':'{profile}':{key} {value_str}");
    plist_buddy(&plist, set_command.as_str())
}

fn plist_buddy(plist: &str, command: &str) -> Result<(), String> {
    let status = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", command, plist])
        .status()
        .map_err(|error| format!("Failed to run PlistBuddy: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("PlistBuddy rejected `{command}`."))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        description_for_detected, hidden_for_detected, install_alacritty_keybinding_at,
        install_vscode_keybinding_at, render_unsupported_terminal_guidance, DetectedTerminal,
        SupportedTerminal,
    };
    use std::fs;

    #[test]
    fn native_terminal_metadata_matches_expected_visibility() {
        let detected = DetectedTerminal::Native("WezTerm");
        assert_eq!(
            description_for_detected(&detected),
            "Install Shift+Enter key binding for newlines"
        );
        assert!(hidden_for_detected(&detected));
    }

    #[test]
    fn apple_terminal_uses_specialized_description() {
        let detected = DetectedTerminal::Supported(SupportedTerminal::AppleTerminal);
        assert_eq!(
            description_for_detected(&detected),
            "Enable Option+Enter key binding for newlines and visual bell"
        );
        assert!(!hidden_for_detected(&detected));
    }

    #[test]
    fn unsupported_terminal_guidance_mentions_supported_reroute() {
        let message = render_unsupported_terminal_guidance("tmux");
        assert!(message.contains("Terminal setup cannot be run from tmux"));
        assert!(message.contains("Run /terminal-setup directly in one of these terminals"));
    }

    #[test]
    fn vscode_install_writes_keybinding_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("Code/User/keybindings.json");
        let message = install_vscode_keybinding_at("VSCode", &path, false).unwrap();
        let contents = fs::read_to_string(path).unwrap();

        assert!(message.contains("Installed VSCode terminal Shift+Enter key binding"));
        assert!(contents.contains("\"shift+enter\""));
        assert!(contents.contains("workbench.action.terminal.sendSequence"));
    }

    #[test]
    fn alacritty_install_appends_binding_block() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("alacritty.toml");
        fs::write(&path, "live_config_reload = true\n").unwrap();

        let message = install_alacritty_keybinding_at(&path).unwrap();
        let contents = fs::read_to_string(path).unwrap();

        assert!(message.contains("Installed Alacritty Shift+Enter key binding"));
        assert!(contents.contains("mods = \"Shift\""));
        assert!(contents.contains("chars = "));
    }
}
