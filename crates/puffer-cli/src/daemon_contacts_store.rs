//! Saved-contact JSON persistence helpers.

use anyhow::{Context, Result};
use puffer_config::ConfigPaths;
use puffer_subscriptions::{normalize_contact_ids, ContactProposal, SavedContact};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Default, Deserialize, Serialize)]
pub(super) struct ContactStoreFile {
    #[serde(default)]
    pub(super) version: u32,
    #[serde(default)]
    pub(super) contacts: Vec<SavedContact>,
    #[serde(default)]
    pub(super) proposals: Vec<ContactProposal>,
}

/// Loads saved contacts from the workspace runtime store.
pub(super) fn load_store(paths: &ConfigPaths) -> Result<ContactStoreFile> {
    let path = contacts_path(paths);
    if !path.exists() {
        return Ok(ContactStoreFile::default());
    }
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(ContactStoreFile::default());
    }
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

/// Saves contacts to the workspace runtime store through a temporary file.
pub(super) fn save_store(paths: &ConfigPaths, store: &ContactStoreFile) -> Result<()> {
    let path = contacts_path(paths);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(store)?)
        .with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))
}

/// Replaces the last inferred contact proposals in the workspace store.
pub(super) fn save_proposals(paths: &ConfigPaths, proposals: Vec<ContactProposal>) -> Result<()> {
    let mut store = load_store(paths)?;
    store.proposals = proposals;
    save_store(paths, &store)
}

/// Removes inferred proposals that overlap saved contact ids.
pub(super) fn prune_proposals_for_contact_ids(
    store: &mut ContactStoreFile,
    contact_ids: &[String],
) {
    let saved_ids = normalize_contact_ids(contact_ids);
    if saved_ids.is_empty() {
        return;
    }
    store.proposals.retain(|proposal| {
        normalize_contact_ids(&proposal.contact_ids)
            .iter()
            .all(|proposal_id| !saved_ids.contains(proposal_id))
    });
}

fn contacts_path(paths: &ConfigPaths) -> PathBuf {
    paths
        .workspace_config_dir
        .join("runtime")
        .join("contacts.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_config::ConfigPaths;
    use std::path::Path;

    #[test]
    fn load_store_defaults_missing_proposals() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_config_paths(temp.path());
        let runtime_dir = paths.workspace_config_dir.join("runtime");
        std::fs::create_dir_all(&runtime_dir).unwrap();
        std::fs::write(
            runtime_dir.join("contacts.json"),
            r#"{"version":1,"contacts":[]}"#,
        )
        .unwrap();

        let store = load_store(&paths).unwrap();

        assert!(store.proposals.is_empty());
    }

    #[test]
    fn save_proposals_preserves_saved_contacts() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_config_paths(temp.path());
        let mut store = ContactStoreFile::default();
        store.contacts.push(SavedContact {
            id: "contact-alice".to_string(),
            name: "Alice".to_string(),
            description: "Project collaborator.".to_string(),
            avatar: None,
            contact_ids: vec!["telegram@alice".to_string()],
        });
        save_store(&paths, &store).unwrap();

        save_proposals(
            &paths,
            vec![ContactProposal {
                name: "Bob".to_string(),
                description: "Frequent project contact.".to_string(),
                avatar: Some("data:image/jpeg;base64,ZmFrZQ==".to_string()),
                contact_ids: vec!["telegram@bob".to_string()],
            }],
        )
        .unwrap();

        let store = load_store(&paths).unwrap();

        assert_eq!(store.contacts.len(), 1);
        assert_eq!(store.proposals.len(), 1);
        assert_eq!(
            store.proposals[0].contact_ids,
            vec!["telegram@bob".to_string()]
        );
    }

    #[test]
    fn prune_proposals_for_contact_ids_removes_overlapping_proposals() {
        let mut store = ContactStoreFile {
            version: 1,
            contacts: Vec::new(),
            proposals: vec![
                ContactProposal {
                    name: "Alice".to_string(),
                    description: "Frequent launch collaborator.".to_string(),
                    avatar: None,
                    contact_ids: vec!["Telegram@Alice".to_string()],
                },
                ContactProposal {
                    name: "Bob".to_string(),
                    description: "Separate support contact.".to_string(),
                    avatar: None,
                    contact_ids: vec!["telegram@bob".to_string()],
                },
            ],
        };

        prune_proposals_for_contact_ids(&mut store, &["telegram@alice".to_string()]);

        assert_eq!(store.proposals.len(), 1);
        assert_eq!(store.proposals[0].name, "Bob");
    }

    fn test_config_paths(root: &Path) -> ConfigPaths {
        ConfigPaths {
            workspace_root: root.to_path_buf(),
            workspace_config_dir: root.join(".puffer"),
            user_config_dir: root.join("user-puffer"),
            builtin_resources_dir: root.join("resources"),
        }
    }
}
