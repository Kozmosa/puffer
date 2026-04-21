use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Identifies one conversation on a specific platform (Telegram chat id,
/// Slack channel id, Discord thread id, email thread id, …).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConversationKey {
    /// Stable platform identifier, e.g. `"telegram"`, `"slack"`, `"discord"`.
    pub platform: String,
    /// Opaque, platform-specific conversation id (chat id, channel id, …).
    pub conversation: String,
}

impl ConversationKey {
    pub fn new(platform: impl Into<String>, conversation: impl Into<String>) -> Self {
        Self {
            platform: platform.into(),
            conversation: conversation.into(),
        }
    }

    /// Compact string form used as a JSON map key on disk.
    fn storage_key(&self) -> String {
        format!("{}::{}", self.platform, self.conversation)
    }
}

/// Persistent map from [`ConversationKey`] to the Puffer `session_id` that
/// represents that conversation on disk.
///
/// Serialized as pretty-printed JSON at a caller-chosen path (typically
/// `~/.puffer/connector_sessions.json`).
#[derive(Debug, Clone, Default)]
pub struct ConversationSessionMap {
    path: Option<PathBuf>,
    entries: BTreeMap<String, Uuid>,
}

impl ConversationSessionMap {
    /// Loads the map from `path`, returning an empty map if the file is
    /// absent. The path is remembered so subsequent `save` calls don't
    /// need it again.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let entries = if path.exists() {
            let raw = fs::read_to_string(&path).with_context(|| {
                format!("failed to read connector session map {}", path.display())
            })?;
            if raw.trim().is_empty() {
                BTreeMap::new()
            } else {
                serde_json::from_str::<BTreeMap<String, Uuid>>(&raw).with_context(|| {
                    format!("failed to parse connector session map {}", path.display())
                })?
            }
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            path: Some(path),
            entries,
        })
    }

    /// Builds an in-memory-only map that will never touch disk.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Returns the session id previously associated with `key`, if any.
    pub fn get(&self, key: &ConversationKey) -> Option<Uuid> {
        self.entries.get(&key.storage_key()).copied()
    }

    /// Inserts (or replaces) the mapping and persists it if the map was
    /// loaded from disk.
    pub fn insert(&mut self, key: &ConversationKey, session_id: Uuid) -> Result<()> {
        self.entries.insert(key.storage_key(), session_id);
        self.save()
    }

    /// Removes the mapping for `key` (if any) and persists the change.
    pub fn remove(&mut self, key: &ConversationKey) -> Result<Option<Uuid>> {
        let previous = self.entries.remove(&key.storage_key());
        self.save()?;
        Ok(previous)
    }

    /// Number of stored mappings; intended for tests and diagnostics.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn save(&self) -> Result<()> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create connector session map parent {}",
                    parent.display()
                )
            })?;
        }
        let raw = serde_json::to_string_pretty(&self.entries)?;
        // Write-then-rename to avoid truncating on crash mid-write.
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, raw)
            .with_context(|| format!("failed to write connector session map {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| {
            format!(
                "failed to replace connector session map {}",
                path.display()
            )
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrips_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let key = ConversationKey::new("telegram", "12345");
        let session_id = Uuid::new_v4();

        let mut map = ConversationSessionMap::load(&path).unwrap();
        assert!(map.is_empty());
        map.insert(&key, session_id).unwrap();

        let reloaded = ConversationSessionMap::load(&path).unwrap();
        assert_eq!(reloaded.get(&key), Some(session_id));
        assert_eq!(reloaded.len(), 1);
    }

    #[test]
    fn in_memory_map_never_writes() {
        let mut map = ConversationSessionMap::in_memory();
        let key = ConversationKey::new("discord", "channel-7");
        map.insert(&key, Uuid::new_v4()).unwrap();
        assert!(map.path().is_none());
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn remove_returns_previous_and_persists() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let key = ConversationKey::new("slack", "C1");
        let session_id = Uuid::new_v4();

        let mut map = ConversationSessionMap::load(&path).unwrap();
        map.insert(&key, session_id).unwrap();
        assert_eq!(map.remove(&key).unwrap(), Some(session_id));

        let reloaded = ConversationSessionMap::load(&path).unwrap();
        assert!(reloaded.get(&key).is_none());
    }

    #[test]
    fn storage_key_disambiguates_platforms() {
        let a = ConversationKey::new("telegram", "42");
        let b = ConversationKey::new("discord", "42");
        assert_ne!(a.storage_key(), b.storage_key());
    }

    #[test]
    fn empty_file_loads_as_empty_map() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        fs::write(&path, "").unwrap();
        let map = ConversationSessionMap::load(&path).unwrap();
        assert!(map.is_empty());
    }
}
