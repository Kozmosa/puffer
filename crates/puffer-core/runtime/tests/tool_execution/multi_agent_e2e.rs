use super::*;
use crate::runtime::teammate_loop::{teammate_registry, IncomingMessage, TeammateMessage};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn run_tool(state: &mut AppState, tool_id: &str, input: Value) -> Result<String, anyhow::Error> {
    let resources = puffer_resources::LoadedResources::default();
    let cwd = state.session.cwd.clone();
    crate::runtime::claude_tools::execute_workflow_tool(
        state, &resources, &cwd, tool_id, input, None,
    )
}

fn parse_result(raw: &str) -> Value {
    serde_json::from_str(raw).expect("tool output should be valid JSON")
}

fn workflow_path(state: &AppState) -> std::path::PathBuf {
    state.session.cwd.join(".puffer/runtime/claude_workflow")
}

/// Sets up a team with one fake running agent and returns (agent_id, team_name).
fn setup_team_with_agent(state: &mut AppState, team: &str, agent: &str) -> String {
    let cwd = state.session.cwd.clone();
    let wf = workflow_path(state);
    fs::create_dir_all(&wf).unwrap();

    run_tool(state, "TeamCreate", json!({"team_name": team})).unwrap();

    let agent_id = format!("{agent}@{team}");
    let output_file = wf.join(format!("{agent}-output.json"));
    fs::write(&output_file, "{}").unwrap();

    let agents = json!({"agents": [{
        "agent_id": &agent_id,
        "name": agent,
        "description": "test agent",
        "prompt": "test",
        "subagent_type": null,
        "model": null,
        "team_name": team,
        "mode": null,
        "isolation": null,
        "cwd": cwd.display().to_string(),
        "status": "running",
        "output_file": output_file.display().to_string(),
    }]});
    fs::write(
        wf.join("agents.json"),
        serde_json::to_string_pretty(&agents).unwrap(),
    )
    .unwrap();

    agent_id
}

/// Also registers the team-lead as an agent so it can receive messages.
fn register_team_lead(state: &AppState, team: &str) {
    let wf = workflow_path(state);
    let lead_output = wf.join("lead-output.json");
    fs::write(&lead_output, "{}").unwrap();

    let mut agents: Value =
        serde_json::from_str(&fs::read_to_string(wf.join("agents.json")).unwrap()).unwrap();
    agents["agents"].as_array_mut().unwrap().push(json!({
        "agent_id": format!("team-lead@{team}"),
        "name": "team-lead",
        "description": "leader",
        "prompt": "",
        "subagent_type": null,
        "model": null,
        "team_name": team,
        "mode": null,
        "isolation": null,
        "cwd": state.session.cwd.display().to_string(),
        "status": "running",
        "output_file": lead_output.display().to_string(),
    }));
    fs::write(
        wf.join("agents.json"),
        serde_json::to_string_pretty(&agents).unwrap(),
    )
    .unwrap();
}

/// Marks all agents as stopped and deletes the team.
fn teardown_team(state: &mut AppState) {
    let wf = workflow_path(state);
    if let Ok(raw) = fs::read_to_string(wf.join("agents.json")) {
        if let Ok(mut agents) = serde_json::from_str::<Value>(&raw) {
            if let Some(arr) = agents["agents"].as_array_mut() {
                for a in arr {
                    a["status"] = json!("stopped");
                }
            }
            let _ = fs::write(
                wf.join("agents.json"),
                serde_json::to_string_pretty(&agents).unwrap(),
            );
        }
    }
    let _ = run_tool(state, "TeamDelete", json!({}));
}

fn read_messages(state: &AppState) -> Vec<Value> {
    let path = workflow_path(state).join("messages.json");
    let raw = fs::read_to_string(path).unwrap_or_else(|_| r#"{"messages":[]}"#.to_string());
    let store: Value = serde_json::from_str(&raw).unwrap();
    store["messages"].as_array().cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Registry tests
// ---------------------------------------------------------------------------

#[test]
fn registry_can_send_incoming_message_via_channel() {
    let (tx, rx) = std::sync::mpsc::channel();
    let id = format!("reg-test-{}", uuid::Uuid::new_v4().simple());

    teammate_registry().lock().unwrap().insert(id.clone(), tx);

    teammate_registry()
        .lock()
        .unwrap()
        .get(&id)
        .unwrap()
        .send(TeammateMessage::Incoming(IncomingMessage {
            from: "leader".into(),
            text: "task A".into(),
        }))
        .unwrap();

    match rx.recv_timeout(Duration::from_secs(1)).unwrap() {
        TeammateMessage::Incoming(m) => {
            assert_eq!(m.from, "leader");
            assert_eq!(m.text, "task A");
        }
        other => panic!("expected Incoming, got {other:?}"),
    }

    teammate_registry().lock().unwrap().remove(&id);
}

#[test]
fn registry_can_send_shutdown_signal() {
    let (tx, rx) = std::sync::mpsc::channel();
    let id = format!("shut-test-{}", uuid::Uuid::new_v4().simple());

    teammate_registry().lock().unwrap().insert(id.clone(), tx);
    teammate_registry()
        .lock()
        .unwrap()
        .get(&id)
        .unwrap()
        .send(TeammateMessage::Shutdown {
            request_id: "r-1".into(),
        })
        .unwrap();

    match rx.recv_timeout(Duration::from_secs(1)).unwrap() {
        TeammateMessage::Shutdown { request_id } => assert_eq!(request_id, "r-1"),
        other => panic!("expected Shutdown, got {other:?}"),
    }

    teammate_registry().lock().unwrap().remove(&id);
}

// ---------------------------------------------------------------------------
// SendMessage — in-process delivery
// ---------------------------------------------------------------------------

#[test]
fn send_message_delivers_via_mpsc_and_persists_to_store() {
    let mut state = temp_state();
    let agent_id = setup_team_with_agent(&mut state, "deliver-team", "worker");

    let (tx, rx) = std::sync::mpsc::channel();
    teammate_registry()
        .lock()
        .unwrap()
        .insert(agent_id.clone(), tx);

    let raw = run_tool(
        &mut state,
        "SendMessage",
        json!({"to": "worker", "summary": "hi", "message": "do task A"}),
    )
    .unwrap();
    let out = parse_result(&raw);

    // Delivered list includes the agent
    assert!(out["delivered"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == &agent_id));

    // Received via mpsc
    match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
        TeammateMessage::Incoming(m) => {
            assert!(
                m.from.contains("team-lead"),
                "from should be team-lead, got {}",
                m.from
            );
            assert_eq!(m.text, "do task A");
        }
        other => panic!("expected Incoming, got {other:?}"),
    }

    // Persisted with from + read fields
    let msgs = read_messages(&state);
    let stored = msgs
        .iter()
        .find(|m| m["to"] == agent_id)
        .expect("should persist");
    assert_eq!(stored["read"], false);
    assert_eq!(stored["from"].as_str().unwrap().is_empty(), false);

    teammate_registry().lock().unwrap().remove(&agent_id);
    teardown_team(&mut state);
}

// ---------------------------------------------------------------------------
// SendMessage — validation
// ---------------------------------------------------------------------------

#[test]
fn send_message_rejects_at_sign_in_recipient() {
    let mut state = temp_state();
    fs::create_dir_all(workflow_path(&state)).unwrap();
    let err = run_tool(
        &mut state,
        "SendMessage",
        json!({"to": "a@b", "summary": "x", "message": "y"}),
    )
    .unwrap_err();
    assert!(err.to_string().contains("do not include @"), "got: {err}");
}

#[test]
fn send_message_requires_summary_for_text_messages() {
    let mut state = temp_state();
    fs::create_dir_all(workflow_path(&state)).unwrap();
    let err = run_tool(
        &mut state,
        "SendMessage",
        json!({"to": "x", "message": "y"}),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("summary is required"),
        "got: {err}"
    );
}

#[test]
fn send_message_blocks_structured_broadcast() {
    let mut state = temp_state();
    fs::create_dir_all(workflow_path(&state)).unwrap();
    let err = run_tool(
        &mut state,
        "SendMessage",
        json!({"to": "*", "message": {"type": "shutdown_request"}}),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("cannot be broadcast"),
        "got: {err}"
    );
}

#[test]
fn send_message_enforces_shutdown_response_target() {
    let mut state = temp_state();
    fs::create_dir_all(workflow_path(&state)).unwrap();
    let err = run_tool(
        &mut state,
        "SendMessage",
        json!({
            "to": "someone",
            "message": {"type": "shutdown_response", "request_id": "x", "approve": true}
        }),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("must be sent to \"team-lead\""),
        "got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Shutdown protocol
// ---------------------------------------------------------------------------

#[test]
fn shutdown_request_generates_tracked_request_id() {
    let mut state = temp_state();
    let _agent_id = setup_team_with_agent(&mut state, "shut-proto", "bot");

    let raw = run_tool(
        &mut state,
        "SendMessage",
        json!({
            "to": "bot",
            "message": {"type": "shutdown_request", "reason": "done"}
        }),
    )
    .unwrap();
    let out = parse_result(&raw);

    assert_eq!(out["success"], true);
    let rid = out["request_id"].as_str().unwrap();
    assert!(
        rid.starts_with("shutdown-"),
        "request_id should start with shutdown-, got {rid}"
    );

    // Stored in shutdown_requests.json
    let sr: Value = serde_json::from_str(
        &fs::read_to_string(workflow_path(&state).join("shutdown_requests.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(sr["requests"][0]["request_id"], rid);

    teardown_team(&mut state);
}

#[test]
fn shutdown_response_approve_completes_roundtrip() {
    let mut state = temp_state();
    let _agent_id = setup_team_with_agent(&mut state, "roundtrip", "helper");
    register_team_lead(&state, "roundtrip");

    // Step 1: request
    let raw = run_tool(
        &mut state,
        "SendMessage",
        json!({
            "to": "helper",
            "message": {"type": "shutdown_request"}
        }),
    )
    .unwrap();
    let rid = parse_result(&raw)["request_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Step 2: response
    let raw = run_tool(
        &mut state,
        "SendMessage",
        json!({
            "to": "team-lead",
            "message": {"type": "shutdown_response", "request_id": &rid, "approve": true}
        }),
    )
    .unwrap();
    assert_eq!(parse_result(&raw)["success"], true);

    // Confirm delivered to team-lead mailbox
    let msgs = read_messages(&state);
    let confirmation = msgs
        .iter()
        .find(|m| m["message"]["type"] == "shutdown_response" && m["to"] == "team-lead");
    assert!(
        confirmation.is_some(),
        "approval should appear in team-lead mailbox"
    );
    assert_eq!(confirmation.unwrap()["message"]["approve"], true);

    teardown_team(&mut state);
}

// ---------------------------------------------------------------------------
// Plan approval
// ---------------------------------------------------------------------------

#[test]
fn plan_approval_delivers_approve_and_reject_with_feedback() {
    let mut state = temp_state();
    let _agent_id = setup_team_with_agent(&mut state, "plan-proto", "dev");

    // Approve
    let raw = run_tool(&mut state, "SendMessage", json!({
        "to": "dev",
        "message": {"type": "plan_approval_response", "request_id": "p1", "approve": true, "feedback": "lgtm"}
    }))
    .unwrap();
    assert_eq!(parse_result(&raw)["success"], true);

    // Reject
    let raw = run_tool(&mut state, "SendMessage", json!({
        "to": "dev",
        "message": {"type": "plan_approval_response", "request_id": "p2", "approve": false, "feedback": "add tests"}
    }))
    .unwrap();
    assert_eq!(parse_result(&raw)["success"], true);

    let msgs = read_messages(&state);
    let plan_msgs: Vec<_> = msgs
        .iter()
        .filter(|m| m["message"]["type"] == "plan_approval_response")
        .collect();
    assert_eq!(plan_msgs.len(), 2);
    assert_eq!(plan_msgs[0]["message"]["approve"], true);
    assert_eq!(plan_msgs[1]["message"]["approve"], false);
    assert_eq!(plan_msgs[1]["message"]["feedback"], "add tests");

    teardown_team(&mut state);
}

// ---------------------------------------------------------------------------
// Task auto-owner
// ---------------------------------------------------------------------------

#[test]
fn task_update_auto_sets_owner_when_team_active() {
    let mut state = temp_state();
    let _agent_id = setup_team_with_agent(&mut state, "owner-team", "bot");

    let raw = run_tool(
        &mut state,
        "TaskCreate",
        json!({"subject": "feat", "description": "d"}),
    )
    .unwrap();
    let task_id = parse_result(&raw)["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let raw = run_tool(
        &mut state,
        "TaskUpdate",
        json!({"taskId": &task_id, "status": "in_progress"}),
    )
    .unwrap();
    let out = parse_result(&raw);
    let fields: Vec<&str> = out["updatedFields"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        fields.contains(&"owner"),
        "updatedFields should include owner, got {fields:?}"
    );

    teardown_team(&mut state);
}
