//! Runtime staging helpers for built-in browser extensions.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// CAPTCHA solver credentials used to preconfigure a bundled browser extension.
#[derive(Clone, PartialEq, Eq)]
pub struct CaptchaExtensionSeed {
    solver_id: String,
    api_key: String,
    base_url: String,
}

impl fmt::Debug for CaptchaExtensionSeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CaptchaExtensionSeed")
            .field("solver_id", &self.solver_id)
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl CaptchaExtensionSeed {
    /// Creates a new built-in CAPTCHA extension seed.
    pub fn new(
        solver_id: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            solver_id: solver_id.into(),
            api_key: api_key.into(),
            base_url: base_url.into(),
        }
    }

    /// Returns the built-in solver id this seed targets.
    pub fn solver_id(&self) -> &str {
        &self.solver_id
    }

    /// Returns the decrypted API key for the extension.
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Returns the configured solver API base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

/// Returns the extension directory to load after applying static seed data.
pub fn stage_builtin_captcha_extension(
    source_dir: &Path,
    stage_root: &Path,
    seed: &CaptchaExtensionSeed,
) -> Result<PathBuf> {
    if seed.solver_id() != "nopecha" {
        return Ok(source_dir.to_path_buf());
    }
    let staged_dir = stage_root.join(seed.solver_id());
    reset_staged_dir(source_dir, &staged_dir)?;
    patch_nopecha_manifest(&staged_dir.join("manifest.json"), seed)?;
    Ok(staged_dir)
}

fn reset_staged_dir(source_dir: &Path, staged_dir: &Path) -> Result<()> {
    if staged_dir.exists() {
        fs::remove_dir_all(staged_dir).with_context(|| {
            format!("reset staged extension directory {}", staged_dir.display())
        })?;
    }
    copy_dir_all(source_dir, staged_dir)
}

fn copy_dir_all(source_dir: &Path, target_dir: &Path) -> Result<()> {
    fs::create_dir_all(target_dir)
        .with_context(|| format!("create extension stage {}", target_dir.display()))?;
    for entry in fs::read_dir(source_dir)
        .with_context(|| format!("read extension source {}", source_dir.display()))?
    {
        let entry = entry.context("read extension source entry")?;
        let source_path = entry.path();
        let target_path = target_dir.join(entry.file_name());
        if entry
            .file_type()
            .with_context(|| format!("read file type for {}", source_path.display()))?
            .is_dir()
        {
            copy_dir_all(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "copy extension file {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn patch_nopecha_manifest(manifest_path: &Path, seed: &CaptchaExtensionSeed) -> Result<()> {
    let contents = fs::read_to_string(manifest_path)
        .with_context(|| format!("read NopeCHA manifest {}", manifest_path.display()))?;
    let mut manifest: Value =
        serde_json::from_str(&contents).context("parse NopeCHA manifest JSON")?;
    let Some(nopecha) = manifest.get_mut("nopecha").and_then(Value::as_object_mut) else {
        bail!("NopeCHA automation manifest is missing the `nopecha` object");
    };
    nopecha.insert("enabled".to_string(), Value::Bool(true));
    nopecha.insert("key".to_string(), Value::String(seed.api_key().to_string()));
    nopecha.insert(
        "_base_api".to_string(),
        Value::String(seed.base_url().to_string()),
    );
    let updated =
        serde_json::to_string_pretty(&manifest).context("serialize NopeCHA staged manifest")?;
    fs::write(manifest_path, updated)
        .with_context(|| format!("write NopeCHA staged manifest {}", manifest_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn stages_nopecha_manifest_with_static_key_config() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/file.js"), "console.log('ok');").unwrap();
        fs::write(
            source.join("manifest.json"),
            serde_json::to_string_pretty(&json!({
                "name": "NopeCHA: CAPTCHA Solver",
                "manifest_version": 3,
                "key": "stable-extension-id-key",
                "nopecha": {
                    "enabled": false,
                    "key": "",
                    "_base_api": "",
                    "recaptcha_auto_solve": true
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let seed = CaptchaExtensionSeed::new("nopecha", "paid-key", "https://api.example.test");

        let staged =
            stage_builtin_captcha_extension(&source, &dir.path().join("stage"), &seed).unwrap();

        assert_eq!(staged, dir.path().join("stage/nopecha"));
        assert_eq!(
            fs::read_to_string(staged.join("nested/file.js")).unwrap(),
            "console.log('ok');"
        );
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(staged.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["key"], "stable-extension-id-key");
        assert_eq!(manifest["nopecha"]["enabled"], true);
        assert_eq!(manifest["nopecha"]["key"], "paid-key");
        assert_eq!(manifest["nopecha"]["_base_api"], "https://api.example.test");
        let source_manifest: Value =
            serde_json::from_str(&fs::read_to_string(source.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(source_manifest["nopecha"]["key"], "");
    }

    #[test]
    fn leaves_runtime_seeded_solvers_at_source_dir() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(&source).unwrap();
        let seed = CaptchaExtensionSeed::new("2captcha", "key", "https://2captcha.test");

        let resolved =
            stage_builtin_captcha_extension(&source, &dir.path().join("stage"), &seed).unwrap();

        assert_eq!(resolved, source);
        assert!(!dir.path().join("stage").exists());
    }
}
