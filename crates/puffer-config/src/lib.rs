use anyhow::Context;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PufferConfig {
    pub app_name: String,
    pub default_model: Option<String>,
    pub default_provider: Option<String>,
    pub theme: String,
    pub mascot: MascotConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MascotConfig {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiConfig {
    pub no_alt_screen: bool,
    pub tmux_golden_mode: bool,
}

impl Default for PufferConfig {
    fn default() -> Self {
        Self {
            app_name: "Puffer Code".to_string(),
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
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigPaths {
    pub workspace_root: PathBuf,
    pub workspace_config_dir: PathBuf,
    pub user_config_dir: PathBuf,
    pub builtin_resources_dir: PathBuf,
}

impl ConfigPaths {
    /// Discovers the standard Puffer config and resource paths from a workspace root.
    pub fn discover(workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        let workspace_config_dir = workspace_root.join(".puffer");
        let user_config_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".puffer");
        let builtin_resources_dir = workspace_root.join("resources");
        Self {
            workspace_root,
            workspace_config_dir,
            user_config_dir,
            builtin_resources_dir,
        }
    }

    /// Returns the workspace-local configuration file path.
    pub fn workspace_config_file(&self) -> PathBuf {
        self.workspace_config_dir.join("config.toml")
    }

    /// Returns the user-level configuration file path.
    pub fn user_config_file(&self) -> PathBuf {
        self.user_config_dir.join("config.toml")
    }

    /// Returns true when the user-level config file already exists.
    pub fn has_user_config(&self) -> bool {
        self.user_config_file().exists()
    }

    /// Returns true when the workspace-level config file already exists.
    pub fn has_workspace_config(&self) -> bool {
        self.workspace_config_file().exists()
    }
}

/// Loads layered Puffer configuration from the user and workspace config files.
pub fn load_config(paths: &ConfigPaths) -> Result<PufferConfig> {
    let mut config = PufferConfig::default();
    if paths.user_config_file().exists() {
        merge_config_file(&mut config, &paths.user_config_file())?;
    }
    if paths.workspace_config_file().exists() {
        merge_config_file(&mut config, &paths.workspace_config_file())?;
    }
    Ok(config)
}

/// Saves the user-level Puffer configuration file.
pub fn save_user_config(paths: &ConfigPaths, config: &PufferConfig) -> Result<()> {
    ensure_workspace_dirs(paths)?;
    write_config_file(&paths.user_config_file(), config)
}

/// Saves the workspace-level Puffer configuration file.
pub fn save_workspace_config(paths: &ConfigPaths, config: &PufferConfig) -> Result<()> {
    ensure_workspace_dirs(paths)?;
    write_config_file(&paths.workspace_config_file(), config)
}

/// Ensures the standard user and workspace configuration directories exist.
pub fn ensure_workspace_dirs(paths: &ConfigPaths) -> Result<()> {
    fs::create_dir_all(&paths.workspace_config_dir).with_context(|| {
        format!(
            "failed to create workspace config dir {}",
            paths.workspace_config_dir.display()
        )
    })?;
    fs::create_dir_all(&paths.user_config_dir).with_context(|| {
        format!(
            "failed to create user config dir {}",
            paths.user_config_dir.display()
        )
    })?;
    Ok(())
}

fn merge_config_file(config: &mut PufferConfig, path: &Path) -> Result<()> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let parsed: PufferConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;
    *config = parsed;
    Ok(())
}

fn write_config_file(path: &Path, config: &PufferConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create config parent dir {}", parent.display())
        })?;
    }
    let raw = toml::to_string_pretty(config)
        .with_context(|| format!("failed to serialize config file {}", path.display()))?;
    fs::write(path, raw).with_context(|| format!("failed to write config file {}", path.display()))
}
