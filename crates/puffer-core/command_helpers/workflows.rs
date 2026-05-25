use crate::{subscription_manager, AppState};
use anyhow::Result;
use puffer_config::ConfigPaths;
use puffer_subscriptions::{builtin_connector_templates, ConnectionRecord, ConnectorTemplate};
use puffer_workflow::{TriggerSpec, WorkflowDefinition, WorkflowRun, WorkflowStore};
use std::fmt::Write as _;

/// Renders a terminal-friendly workflow, connection, and connector summary.
pub(crate) fn handle_workflows_command(state: &AppState, args: &str) -> Result<String> {
    let paths = ConfigPaths::discover(&state.cwd);
    let store = WorkflowStore::new(&paths.workspace_config_dir);
    let workflows = store.list()?;
    let runs = store.list_runs()?;
    let connector_context = connector_context();
    let mode = args.trim();

    let mut out = String::new();
    match mode {
        "" | "show" | "list" => {
            write_header(&mut out, &paths, &workflows, &runs, &connector_context);
            write_workflows(&mut out, &workflows, &runs);
            write_connections(&mut out, &connector_context);
            write_connectors(&mut out, &connector_context, false);
        }
        "connections" | "connection" => {
            write_header(&mut out, &paths, &workflows, &runs, &connector_context);
            write_connections(&mut out, &connector_context);
        }
        "connectors" | "connector" => {
            write_header(&mut out, &paths, &workflows, &runs, &connector_context);
            write_connectors(&mut out, &connector_context, true);
        }
        "runs" | "run" => {
            write_header(&mut out, &paths, &workflows, &runs, &connector_context);
            write_runs(&mut out, &runs);
        }
        _ => {
            out.push_str(
                "Usage: /workflows [list|connections|connectors|runs]\n\
                 Tip: use /connect to add a connector connection, then select it as a workflow trigger.",
            );
        }
    }
    Ok(out)
}

struct ConnectorContext {
    connectors: Vec<ConnectorTemplate>,
    connections: Vec<ConnectionRecord>,
    error: Option<String>,
}

fn connector_context() -> ConnectorContext {
    match subscription_manager() {
        Ok(manager) => {
            let error = manager
                .refresh_connection_consumers()
                .err()
                .map(|error| error.to_string());
            ConnectorContext {
                connectors: manager.connector_store().list_with_builtins(),
                connections: manager.connection_store().list(),
                error,
            }
        }
        Err(error) => ConnectorContext {
            connectors: builtin_connector_templates(),
            connections: Vec::new(),
            error: Some(error.to_string()),
        },
    }
}

fn write_header(
    out: &mut String,
    paths: &ConfigPaths,
    workflows: &[WorkflowDefinition],
    runs: &[WorkflowRun],
    context: &ConnectorContext,
) {
    let _ = writeln!(out, "Workflow dashboard");
    let _ = writeln!(out, "workspace={}", paths.workspace_root.display());
    let _ = writeln!(
        out,
        "workflows={} runs={} connections={} connectors={}",
        workflows.len(),
        runs.len(),
        context.connections.len(),
        context.connectors.len()
    );
    if let Some(error) = &context.error {
        let _ = writeln!(out, "connector_runtime={}", first_line(error));
    }
}

fn write_workflows(out: &mut String, workflows: &[WorkflowDefinition], runs: &[WorkflowRun]) {
    out.push_str("\nWorkflows\n");
    if workflows.is_empty() {
        out.push_str("- none registered\n");
        return;
    }
    for workflow in workflows {
        let latest = runs
            .iter()
            .filter(|run| run.workflow_slug == workflow.slug)
            .max_by_key(|run| run.idx);
        let latest_text = latest
            .map(|run| format!("latest=#{} {:?}", run.idx, run.status))
            .unwrap_or_else(|| "latest=none".to_string());
        let _ = writeln!(
            out,
            "- {} [{}] trigger={} nodes={} {}",
            workflow.slug,
            if workflow.enabled {
                "enabled"
            } else {
                "paused"
            },
            trigger_label(&workflow.trigger),
            workflow.pipeline.nodes.len(),
            latest_text
        );
    }
}

fn write_connections(out: &mut String, context: &ConnectorContext) {
    out.push_str("\nConnections\n");
    if context.connections.is_empty() {
        out.push_str("- none configured; run /connect <connector-slug> <connection-name>\n");
        return;
    }
    for connection in &context.connections {
        let consumer = if connection.has_consumer {
            "consumer=active"
        } else {
            "consumer=idle"
        };
        let _ = writeln!(
            out,
            "- {} connector={} state={:?} {} {}",
            connection.slug,
            connection.connector_slug,
            connection.state,
            consumer,
            connection.description
        );
    }
}

fn write_connectors(out: &mut String, context: &ConnectorContext, include_all: bool) {
    out.push_str("\nConnectors\n");
    let connectors = context
        .connectors
        .iter()
        .filter(|connector| include_all || connector.can_subscribe)
        .collect::<Vec<_>>();
    if connectors.is_empty() {
        out.push_str("- none available\n");
        return;
    }
    for connector in connectors {
        let mut capabilities = Vec::new();
        if connector.requires_auth {
            capabilities.push("auth");
        }
        if connector.can_subscribe {
            capabilities.push("events");
        }
        if connector.can_proxy_agent {
            capabilities.push("proxy");
        }
        if !connector.actions.is_empty() {
            capabilities.push("actions");
        }
        let _ = writeln!(
            out,
            "- {} [{}] {}",
            connector.slug,
            capabilities.join(","),
            connector.description
        );
    }
}

fn write_runs(out: &mut String, runs: &[WorkflowRun]) {
    out.push_str("\nRuns\n");
    if runs.is_empty() {
        out.push_str("- none recorded\n");
        return;
    }
    for run in runs.iter().take(20) {
        let _ = writeln!(
            out,
            "- #{} {} {:?} nodes={}{}",
            run.idx,
            run.workflow_slug,
            run.status,
            run.nodes.len(),
            run.error
                .as_deref()
                .map(|error| format!(" error={}", first_line(error)))
                .unwrap_or_default()
        );
    }
}

fn trigger_label(trigger: &TriggerSpec) -> String {
    match trigger {
        TriggerSpec::Cron { cron } => format!("cron:{cron}"),
        TriggerSpec::Subscription { source_topic, .. } => format!("connection:{source_topic}"),
        TriggerSpec::Connection {
            connection_slug, ..
        } => format!("connection:{connection_slug}"),
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text)
}
