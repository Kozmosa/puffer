use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopPinState {
    #[serde(default)]
    pub(crate) pinned_agent_ids: Vec<String>,
    #[serde(default)]
    pub(crate) pinned_workspace_paths: Vec<String>,
}

/// Loads daemon-persisted desktop pin state from the user config directory.
pub(crate) fn load_pin_state(user_config_dir: &Path) -> Result<DesktopPinState> {
    let path = pin_state_path(user_config_dir);
    if !path.exists() {
        return Ok(DesktopPinState::default());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let state: DesktopPinState = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(normalize_state(state))
}

/// Persists one agent or workspace pin and returns the updated state.
pub(crate) fn set_pin_state(
    user_config_dir: &Path,
    kind: &str,
    id: &str,
    pinned: bool,
) -> Result<DesktopPinState> {
    let mut state = load_pin_state(user_config_dir)?;
    match kind {
        "agent" => set_membership(&mut state.pinned_agent_ids, id, pinned),
        "workspace" => set_membership(&mut state.pinned_workspace_paths, id, pinned),
        other => anyhow::bail!("unsupported pin kind `{other}`"),
    }
    save_pin_state(user_config_dir, &state)?;
    Ok(state)
}

fn save_pin_state(user_config_dir: &Path, state: &DesktopPinState) -> Result<()> {
    let path = pin_state_path(user_config_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(&normalize_state(state.clone()))?;
    std::fs::write(&path, raw).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn pin_state_path(user_config_dir: &Path) -> std::path::PathBuf {
    user_config_dir.join("desktop_state.json")
}

fn set_membership(values: &mut Vec<String>, id: &str, pinned: bool) {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return;
    }
    values.retain(|value| value != trimmed);
    if pinned {
        values.insert(0, trimmed.to_string());
    }
}

fn normalize_state(mut state: DesktopPinState) -> DesktopPinState {
    dedupe_nonempty(&mut state.pinned_agent_ids);
    dedupe_nonempty(&mut state.pinned_workspace_paths);
    state
}

fn dedupe_nonempty(values: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    values.retain(|value| {
        let trimmed = value.trim();
        !trimmed.is_empty() && seen.insert(trimmed.to_string())
    });
}
