use anyhow::{Context, Result};
use puffer_resources::{McpServerSpec, PluginSpec};
use serde::de::DeserializeOwned;
use std::fs;
use std::path::{Path, PathBuf};

/// Returns a YAML file in `dir` whose deserialized id matches `id`.
pub(crate) fn find_yaml_file_by_id<T>(dir: &Path, id: &str) -> Result<Option<PathBuf>>
where
    T: DeserializeOwned + HasStableId,
{
    if !dir.exists() {
        return Ok(None);
    }
    for entry in sorted_dir_entries(dir)? {
        let path = entry.path();
        if !is_yaml_path(&path) {
            continue;
        }
        let value: T = serde_yaml::from_str(&fs::read_to_string(&path)?)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        if value.stable_id() == id {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// Returns stable, path-sorted directory entries for `dir`.
pub(crate) fn sorted_dir_entries(dir: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("failed to read resource dir {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("failed to list resource dir {}", dir.display()))?;
    entries.sort_by(|left, right| left.path().cmp(&right.path()));
    Ok(entries)
}

/// Returns true when `path` is a `.yaml` or `.yml` file.
pub(crate) fn is_yaml_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("yaml" | "yml")
    )
}

/// Returns true when `path` is a disabled YAML sidecar.
pub(crate) fn is_disabled_yaml_path(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(name) if name.ends_with(".yaml.disabled") || name.ends_with(".yml.disabled")
    )
}

/// Returns the disabled sidecar path for a YAML manifest.
pub(crate) fn disabled_variant(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.disabled", path.display()))
}

/// Returns the enabled YAML path for a disabled sidecar.
pub(crate) fn enabled_variant(path: &Path) -> PathBuf {
    let raw = path.display().to_string();
    PathBuf::from(raw.trim_end_matches(".disabled"))
}

/// Returns the conventional manifest path for a plugin id in `dir`.
pub(crate) fn plugin_manifest_path(dir: &Path, plugin_id: &str) -> PathBuf {
    dir.join(format!("{plugin_id}.yaml"))
}

/// Removes `path` when it exists.
pub(crate) fn remove_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub(crate) trait HasStableId {
    fn stable_id(&self) -> &str;
}

impl HasStableId for PluginSpec {
    fn stable_id(&self) -> &str {
        &self.id
    }
}

impl HasStableId for McpServerSpec {
    fn stable_id(&self) -> &str {
        &self.id
    }
}
