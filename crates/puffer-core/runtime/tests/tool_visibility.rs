use super::*;
use puffer_resources::load_resources;
use serde_json::json;
use std::path::{Path, PathBuf};

const SLEEP_TOOL_DESCRIPTION: &str =
    "Wait for a specified duration. The user can interrupt the sleep at any time.\n\nUse this when the user tells you to sleep or rest, when you have nothing to do, or when you're waiting for something.\n\nYou may receive <tick> prompts — these are periodic check-ins. Look for useful work to do before sleeping.\n\nYou can call this concurrently with other tools — it won't interfere with them.\n\nPrefer this over `Bash(sleep ...)` — it doesn't hold a shell process.\n\nEach wake-up costs an API call, but the prompt cache expires after 5 minutes of inactivity — balance accordingly.";

#[test]
fn sleep_tool_is_visible_to_anthropic_and_openai_tool_builders() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);

    let anthropic = anthropic_tool_definitions(&registry, None).unwrap();
    let anthropic_sleep = anthropic
        .iter()
        .find(|definition| definition["name"] == json!("Sleep"))
        .expect("Sleep tool definition");
    assert_eq!(
        anthropic_sleep["description"],
        json!(SLEEP_TOOL_DESCRIPTION)
    );
    assert_eq!(
        anthropic_sleep["input_schema"]["required"],
        json!(["duration_ms"])
    );

    let openai = openai_tool_definitions(&registry, None, false).unwrap();
    let openai_sleep = openai
        .iter()
        .find(|definition| definition.name == "Sleep")
        .expect("Sleep tool definition");
    assert_eq!(openai_sleep.description, SLEEP_TOOL_DESCRIPTION);
    assert_eq!(openai_sleep.parameters["required"], json!(["duration_ms"]));
}

#[test]
fn bundled_resources_register_sleep_tool() {
    let resources = bundled_resources();
    let registry = ToolRegistry::from_resources(&resources);
    let definition = registry.definition("Sleep").expect("Sleep tool definition");

    assert_eq!(definition.handler, "runtime:sleep");
    assert_eq!(definition.description, SLEEP_TOOL_DESCRIPTION);
}

fn bundled_resources() -> LoadedResources {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths {
        workspace_root: temp.path().join("workspace"),
        workspace_config_dir: temp.path().join("workspace/.puffer"),
        user_config_dir: temp.path().join("user"),
        builtin_resources_dir: root.join("resources"),
    };
    load_resources(&paths).unwrap()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}
