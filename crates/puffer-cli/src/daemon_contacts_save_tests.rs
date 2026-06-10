use super::*;
use puffer_config::ConfigPaths;
use puffer_subscriptions::ContactProposal;
use serde_json::json;
use std::path::Path;

#[test]
fn save_contact_normalizes_ids() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_config_paths(temp.path());
    let result = handle_contacts_save(
        &paths,
        &json!({
            "name": "Alice",
            "description": "Project collaborator",
            "contact_ids": ["Google@Alice@Example.COM", "telegram@@alice"]
        }),
    )
    .unwrap();

    let contact = &result["contacts"].as_array().unwrap()[0];
    assert_eq!(contact["contact_ids"][0], "google@alice@example.com");
    assert_eq!(contact["contact_ids"][1], "telegram@alice");
}

#[test]
fn save_contact_survives_fresh_store_load() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_config_paths(temp.path());
    handle_contacts_save(
        &paths,
        &json!({
            "name": "Alice",
            "description": "Project collaborator",
            "contact_ids": ["telegram@alice"]
        }),
    )
    .unwrap();

    let next_paths = test_config_paths(temp.path());
    let result = handle_contacts_list(&next_paths, &json!({ "limit": 10 })).unwrap();
    let contacts = result["contacts"].as_array().unwrap();

    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0]["name"], "Alice");
    assert_eq!(contacts[0]["contact_ids"][0], "telegram@alice");
}

#[test]
fn save_contact_prunes_saved_inferred_proposals() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_config_paths(temp.path());
    save_proposals(
        &paths,
        vec![ContactProposal {
            name: "Alice".to_string(),
            description: "Frequent project contact.".to_string(),
            avatar: None,
            contact_ids: vec!["telegram@alice".to_string()],
        }],
    )
    .unwrap();

    let result = handle_contacts_save(
        &paths,
        &json!({ "name": "Alice", "contact_ids": ["telegram@alice"] }),
    )
    .unwrap();

    assert_eq!(result["contacts"][0]["description"], "");
    assert!(result["proposals"].as_array().unwrap().is_empty());
}

fn test_config_paths(root: &Path) -> ConfigPaths {
    ConfigPaths {
        workspace_root: root.to_path_buf(),
        workspace_config_dir: root.join(".puffer"),
        user_config_dir: root.join("user-puffer"),
        builtin_resources_dir: root.join("resources"),
    }
}
