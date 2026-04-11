use super::*;
use puffer_resources::LoadedResources;

fn ensure_workflow_dir(state: &AppState) {
    let cwd = state.session.cwd.as_path();
    let workflow = cwd.join(".puffer").join("runtime").join("claude_workflow");
    fs::create_dir_all(&workflow).unwrap();
}

fn run_tool(state: &mut AppState, tool_id: &str, input: Value) -> Result<String, anyhow::Error> {
    let resources = LoadedResources::default();
    let cwd = state.session.cwd.clone();
    crate::runtime::claude_tools::execute_workflow_tool(
        state, &resources, &cwd, tool_id, input, None,
    )
}

#[test]
fn agent_team_e2e_full_lifecycle() {
    let mut state = temp_state();
    ensure_workflow_dir(&state);
    let cwd = state.session.cwd.clone();

    // ── Step 1: TeamCreate ──────────────────────────────────────
    let result = run_tool(
        &mut state,
        "TeamCreate",
        json!({
            "team_name": "alpha-team",
            "description": "E2E test team"
        }),
    )
    .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["team_name"], "alpha-team");
    assert!(parsed["lead_agent_id"]
        .as_str()
        .unwrap()
        .contains("team-lead"));
    assert_eq!(state.active_team_name.as_deref(), Some("alpha-team"));

    // Team directory created
    let team_dir = cwd.join(".puffer/runtime/claude_workflow/teams/alpha-team");
    assert!(team_dir.exists());

    // Task list was reset
    let task_file = cwd.join(".puffer/runtime/claude_workflow/team_tasks/alpha-team/tasks.json");
    assert!(task_file.exists());
    let task_store: Value = serde_json::from_str(&fs::read_to_string(&task_file).unwrap()).unwrap();
    assert_eq!(task_store["tasks"].as_array().unwrap().len(), 0);

    // ── Step 2: SendMessage validation ──────────────────────────
    // @ rejection
    let err = run_tool(
        &mut state,
        "SendMessage",
        json!({
            "to": "user@team",
            "summary": "test",
            "message": "hello"
        }),
    );
    assert!(err.unwrap_err().to_string().contains("do not include @"));

    // Empty summary for string message
    let err = run_tool(
        &mut state,
        "SendMessage",
        json!({
            "to": "someone",
            "message": "hello"
        }),
    );
    assert!(err.unwrap_err().to_string().contains("summary is required"));

    // Structured broadcast rejection
    let err = run_tool(
        &mut state,
        "SendMessage",
        json!({
            "to": "*",
            "message": { "type": "shutdown_request" }
        }),
    );
    assert!(err
        .unwrap_err()
        .to_string()
        .contains("structured messages cannot be broadcast"));

    // shutdown_response must target team-lead
    let err = run_tool(
        &mut state,
        "SendMessage",
        json!({
            "to": "researcher",
            "message": {
                "type": "shutdown_response",
                "request_id": "test-123",
                "approve": true
            }
        }),
    );
    assert!(err
        .unwrap_err()
        .to_string()
        .contains("shutdown_response must be sent to \"team-lead\""));

    // ── Step 3: Shutdown request protocol ───────────────────────
    // Register a fake agent
    let agents_json = json!({
        "agents": [{
            "agent_id": "researcher@alpha-team",
            "name": "researcher",
            "description": "test",
            "prompt": "test",
            "subagent_type": null,
            "model": null,
            "team_name": "alpha-team",
            "mode": null,
            "isolation": null,
            "cwd": cwd.display().to_string(),
            "status": "running",
            "output_file": cwd.join("researcher-output.json").display().to_string()
        }]
    });
    fs::write(
        cwd.join(".puffer/runtime/claude_workflow/agents.json"),
        serde_json::to_string_pretty(&agents_json).unwrap(),
    )
    .unwrap();
    fs::write(cwd.join("researcher-output.json"), "{}").unwrap();

    // Send shutdown_request — should generate request_id
    let result = run_tool(
        &mut state,
        "SendMessage",
        json!({
            "to": "researcher",
            "message": {
                "type": "shutdown_request",
                "reason": "test complete"
            }
        }),
    )
    .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["success"], true);
    let request_id = parsed["request_id"].as_str().unwrap();
    assert!(request_id.starts_with("shutdown-"));

    // Shutdown request stored
    let sr_path = cwd.join(".puffer/runtime/claude_workflow/shutdown_requests.json");
    assert!(sr_path.exists());
    let sr: Value = serde_json::from_str(&fs::read_to_string(&sr_path).unwrap()).unwrap();
    assert_eq!(sr["requests"][0]["request_id"], request_id);

    // Messages have from/read fields
    let msgs: Value = serde_json::from_str(
        &fs::read_to_string(cwd.join(".puffer/runtime/claude_workflow/messages.json")).unwrap(),
    )
    .unwrap();
    let last_msg = msgs["messages"].as_array().unwrap().last().unwrap();
    assert!(!last_msg["from"].as_str().unwrap().is_empty());
    assert_eq!(last_msg["read"], false);

    // ── Step 4: TaskCreate + auto-owner ─────────────────────────
    let result = run_tool(
        &mut state,
        "TaskCreate",
        json!({
            "subject": "Test task",
            "description": "A test task"
        }),
    )
    .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let task_id = parsed["task"]["id"].as_str().unwrap().to_string();

    // Mark in_progress — should auto-set owner
    let result = run_tool(
        &mut state,
        "TaskUpdate",
        json!({
            "taskId": task_id,
            "status": "in_progress"
        }),
    )
    .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert!(parsed["updatedFields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "owner"));

    // ── Step 5: TeamDelete ──────────────────────────────────────
    // Mark agent as stopped first
    let mut agents: Value = serde_json::from_str(
        &fs::read_to_string(cwd.join(".puffer/runtime/claude_workflow/agents.json")).unwrap(),
    )
    .unwrap();
    agents["agents"][0]["status"] = json!("stopped");
    fs::write(
        cwd.join(".puffer/runtime/claude_workflow/agents.json"),
        serde_json::to_string_pretty(&agents).unwrap(),
    )
    .unwrap();

    let result = run_tool(&mut state, "TeamDelete", json!({})).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["success"], true);
    assert!(state.active_team_name.is_none());
    assert!(!team_dir.exists());
}
