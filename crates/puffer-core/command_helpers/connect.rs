use crate::runtime::claude_tools::execute_workflow_tool;
use crate::{AppState, TurnExecution};
use anyhow::{anyhow, bail, Context, Result};
use puffer_resources::LoadedResources;
use serde_json::{json, Value};

use super::common::render_svg_qr_data_uri;

mod catalog;
mod gcal_browser;
mod gmail_browser;
mod lark_cli;
mod serve_config;
use lark_cli::*;

const TELEGRAM_QR_APPROVAL_ATTEMPTS: usize = 3;
const TELEGRAM_QR_APPROVAL_CHECK_SECONDS: u64 = 10;

/// Runs the deterministic `/connect` connector-auth flow without a provider turn.
pub fn execute_connect_flow(
    state: &mut AppState,
    resources: &LoadedResources,
    args: &str,
) -> Result<TurnExecution> {
    let target = parse_or_ask_target(state, resources, args)?;
    let result = match target.connector_slug.as_str() {
        "telegram-login" => connect_telegram(state, resources, &target.connection_name)?,
        "slack-app" => connect_slack_app(state, resources, &target.connection_name)?,
        "slack-login" => connect_slack_login(state, resources, &target.connection_name)?,
        "lark-login" | "lark-bot" => connect_lark_cli(
            state,
            resources,
            &target.connector_slug,
            &target.connection_name,
        )?,
        "gmail-browser" => {
            gmail_browser::connect_gmail_browser(state, resources, &target.connection_name)?
        }
        "gcal-browser" => {
            gcal_browser::connect_gcal_browser(state, resources, &target.connection_name)?
        }
        "email" => connect_email(state, resources, &target.connection_name)?,
        "telegram-bot" => {
            serve_config::connect_telegram_bot(state, resources, &target.connection_name)?
        }
        "discord-bot" => {
            serve_config::connect_discord_bot(state, resources, &target.connection_name)?
        }
        "matrix-bot" => {
            serve_config::connect_matrix_bot(state, resources, &target.connection_name)?
        }
        _ => connect_generic(state, resources, &target)?,
    };
    Ok(TurnExecution {
        assistant_text: result.summary,
        tool_invocations: Vec::new(),
        reflection_traces: Vec::new(),
    })
}

struct ConnectTarget {
    connector_slug: String,
    connection_name: String,
}

struct ConnectResult {
    summary: String,
}

fn parse_or_ask_target(
    state: &mut AppState,
    resources: &LoadedResources,
    args: &str,
) -> Result<ConnectTarget> {
    let mut parts = args.split_whitespace();
    let mut connector_slug = parts.next().unwrap_or_default().to_string();
    let mut connection_name = parts.next().unwrap_or_default().to_string();
    let extra = parts.collect::<Vec<_>>().join(" ");

    connector_slug = if connector_slug.is_empty() {
        catalog::ask_connector_slug(state, resources)?
    } else {
        catalog::resolve_connector_slug(state, resources, &connector_slug)?
    };
    if !extra.is_empty() {
        bail!("Usage: /connect <connector-slug> <connection-name>");
    }
    if connection_name.is_empty() {
        connection_name = puffer_subscriptions::suggested_connection_slug(&connector_slug);
    }

    Ok(ConnectTarget {
        connector_slug,
        connection_name,
    })
}

fn connect_telegram(
    state: &mut AppState,
    resources: &LoadedResources,
    connection: &str,
) -> Result<ConnectResult> {
    let method = ask_choice(
        state,
        resources,
        "Method",
        "How should Telegram authenticate this connection?",
        &[
            (
                "Telegram Desktop import",
                "Import an existing local Telegram Desktop session.",
            ),
            (
                "QR login",
                "Approve a login URL from a logged-in Telegram app.",
            ),
            (
                "Phone login",
                "Request a Telegram login code for a phone number.",
            ),
        ],
    )?;
    let output = match method.as_str() {
        "Telegram Desktop import" => call_tool(
            state,
            resources,
            "Telegram",
            json!({
                "action": "import_desktop",
                "connection_slug": connection
            }),
        )?,
        "QR login" => telegram_qr_login(state, resources, connection)?,
        "Phone login" => telegram_phone_login(state, resources, connection)?,
        _ => bail!("unsupported Telegram auth method `{method}`"),
    };
    Ok(summary("telegram-login", connection, &method, &output))
}

fn telegram_qr_login(
    state: &mut AppState,
    resources: &LoadedResources,
    connection: &str,
) -> Result<Value> {
    let mut output = call_tool(
        state,
        resources,
        "Telegram",
        json!({
            "action": "login_qr",
            "connection_slug": connection
        }),
    )?;
    for _ in 0..TELEGRAM_QR_APPROVAL_ATTEMPTS {
        if status(&output) != Some("qr_pending") {
            return submit_telegram_password_if_needed(state, resources, connection, output);
        }
        let url = output
            .pointer("/payload/url")
            .and_then(Value::as_str)
            .unwrap_or("<missing QR URL>");
        let question = telegram_qr_approval_question(url);
        let answer = ask_choice(
            state,
            resources,
            "Approve",
            &question,
            &[
                ("Approved", "I approved the login request."),
                ("Cancel", "Stop this login attempt."),
            ],
        )?;
        if answer != "Approved" {
            bail!("Telegram QR login cancelled");
        }
        output = call_tool(
            state,
            resources,
            "Telegram",
            telegram_qr_wait_input(connection),
        )?;
    }
    bail!(
        "Telegram QR login was not approved after {TELEGRAM_QR_APPROVAL_ATTEMPTS} checks; restart setup to try again"
    )
}

fn telegram_qr_approval_question(url: &str) -> String {
    let qr_body = match render_svg_qr_data_uri(url) {
        Some(data_uri) => format!("![Telegram QR code]({data_uri})\n\n{url}"),
        None => url.to_string(),
    };
    format!("Approve this Telegram QR login URL from a logged-in Telegram app.\n\n{qr_body}")
}

fn telegram_qr_wait_input(connection: &str) -> Value {
    json!({
        "action": "login_qr_wait",
        "connection_slug": connection,
        "timeout_seconds": TELEGRAM_QR_APPROVAL_CHECK_SECONDS
    })
}

fn telegram_phone_login(
    state: &mut AppState,
    resources: &LoadedResources,
    connection: &str,
) -> Result<Value> {
    let phone = ask_input(
        state,
        resources,
        "Phone",
        "What E.164 phone number should Telegram use?",
    )?;
    let mut output = call_tool(
        state,
        resources,
        "Telegram",
        json!({
            "action": "login_start",
            "connection_slug": connection,
            "phone": phone
        }),
    )?;
    if status(&output) == Some("awaiting_code") {
        let code = ask_input(
            state,
            resources,
            "Code",
            "What login code did Telegram send?",
        )?;
        output = call_tool(
            state,
            resources,
            "Telegram",
            json!({
                "action": "login_submit_code",
                "connection_slug": connection,
                "code": code
            }),
        )?;
    }
    if status(&output) == Some("awaiting_password") {
        output = submit_telegram_password_if_needed(state, resources, connection, output)?;
    }
    Ok(output)
}

fn submit_telegram_password_if_needed(
    state: &mut AppState,
    resources: &LoadedResources,
    connection: &str,
    mut output: Value,
) -> Result<Value> {
    if status(&output) == Some("awaiting_password") {
        let password = ask_input(
            state,
            resources,
            "Password",
            "What is the Telegram 2FA cloud password?",
        )?;
        output = call_tool(
            state,
            resources,
            "Telegram",
            json!({
                "action": "login_submit_password",
                "connection_slug": connection,
                "password": password
            }),
        )?;
    }
    Ok(output)
}

fn connect_slack_app(
    state: &mut AppState,
    resources: &LoadedResources,
    connection: &str,
) -> Result<ConnectResult> {
    let bot_token = ask_input(
        state,
        resources,
        "Bot Token",
        "What Slack bot token should Puffer use?",
    )?;
    let app_token = ask_input(
        state,
        resources,
        "App Token",
        "What Slack app-level token should Puffer use?",
    )?;
    let output = call_tool(
        state,
        resources,
        "Slack",
        json!({
            "action": "configure_app",
            "connection_slug": connection,
            "bot_token": bot_token,
            "app_token": app_token
        }),
    )?;
    Ok(summary("slack-app", connection, "App credentials", &output))
}

fn connect_slack_login(
    state: &mut AppState,
    resources: &LoadedResources,
    connection: &str,
) -> Result<ConnectResult> {
    let method = ask_choice(
        state,
        resources,
        "Method",
        "How should Slack authenticate this connection?",
        &[
            ("OAuth token", "Use an existing Slack OAuth token."),
            (
                "Local app import",
                "Import local Slack app browser credentials.",
            ),
            ("Browser tokens", "Use xoxd and xoxc browser tokens."),
        ],
    )?;
    let output = match method.as_str() {
        "OAuth token" => {
            let token = ask_input(
                state,
                resources,
                "Token",
                "What Slack OAuth token should Puffer use?",
            )?;
            call_tool(
                state,
                resources,
                "Slack",
                json!({
                    "action": "login_token",
                    "connection_slug": connection,
                    "token": token
                }),
            )?
        }
        "Local app import" => call_tool(
            state,
            resources,
            "Slack",
            json!({
                "action": "import_local",
                "connection_slug": connection
            }),
        )?,
        "Browser tokens" => {
            let workspace_url = ask_input(
                state,
                resources,
                "Workspace",
                "What Slack workspace URL should Puffer use?",
            )?;
            let xoxd = ask_input(
                state,
                resources,
                "xoxd",
                "What Slack xoxd browser cookie should Puffer use?",
            )?;
            let xoxc = ask_input(
                state,
                resources,
                "xoxc",
                "What Slack xoxc browser token should Puffer use?",
            )?;
            call_tool(
                state,
                resources,
                "Slack",
                json!({
                    "action": "login_browser",
                    "connection_slug": connection,
                    "workspace_url": workspace_url,
                    "xoxd_token": xoxd,
                    "xoxc_token": xoxc
                }),
            )?
        }
        _ => bail!("unsupported Slack login method `{method}`"),
    };
    Ok(summary("slack-login", connection, &method, &output))
}

fn connect_email(
    state: &mut AppState,
    resources: &LoadedResources,
    connection: &str,
) -> Result<ConnectResult> {
    let imap_host = ask_input(
        state,
        resources,
        "IMAP",
        "What IMAP host should Puffer use?",
    )?;
    let imap_port = ask_port(
        state,
        resources,
        "IMAP Port",
        "What IMAP port should Puffer use?",
    )?;
    let smtp_host = ask_input(
        state,
        resources,
        "SMTP",
        "What SMTP host should Puffer use?",
    )?;
    let smtp_port = ask_port(
        state,
        resources,
        "SMTP Port",
        "What SMTP port should Puffer use?",
    )?;
    let username = ask_input(
        state,
        resources,
        "Username",
        "What email username should Puffer use?",
    )?;
    let password = ask_input(
        state,
        resources,
        "Password",
        "What email password or app password should Puffer use?",
    )?;
    let from_address = ask_input(
        state,
        resources,
        "From",
        "What from address should outbound email use?",
    )?;
    let output = call_tool(
        state,
        resources,
        "Email",
        json!({
            "action": "configure",
            "imap_host": imap_host,
            "imap_port": imap_port,
            "smtp_host": smtp_host,
            "smtp_port": smtp_port,
            "username": username,
            "password": password,
            "from_address": from_address
        }),
    )?;
    let connection_output = call_tool(
        state,
        resources,
        "ConnectionCreate",
        json!({
            "slug": connection,
            "connector_slug": "email",
            "description": "Email connection configured by /connect"
        }),
    )?;
    let mut merged = output;
    merged["connection"] = connection_output;
    Ok(summary("email", connection, "Email credentials", &merged))
}

fn connect_generic(
    state: &mut AppState,
    resources: &LoadedResources,
    target: &ConnectTarget,
) -> Result<ConnectResult> {
    let output = call_tool(state, resources, "ConnectorList", json!({}))?;
    let connector = output
        .get("connectors")
        .and_then(Value::as_array)
        .and_then(|connectors| {
            connectors.iter().find(|connector| {
                connector.get("connector_slug").and_then(Value::as_str)
                    == Some(target.connector_slug.as_str())
            })
        })
        .ok_or_else(|| anyhow!("connector `{}` not found", target.connector_slug))?;
    if connector
        .get("requires_auth")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        bail!(
            "/connect does not yet have a deterministic auth flow for connector `{}`",
            target.connector_slug
        );
    }
    let output = call_tool(
        state,
        resources,
        "ConnectionCreate",
        json!({
            "slug": target.connection_name,
            "connector_slug": target.connector_slug,
            "description": "Connection registered by /connect"
        }),
    )?;
    Ok(summary(
        &target.connector_slug,
        &target.connection_name,
        "No-auth connection",
        &output,
    ))
}

fn ask_port(
    state: &mut AppState,
    resources: &LoadedResources,
    header: &str,
    question: &str,
) -> Result<u16> {
    let value = ask_input(state, resources, header, question)?;
    value
        .parse::<u16>()
        .with_context(|| format!("`{value}` is not a valid TCP port"))
}

fn ask_input(
    state: &mut AppState,
    resources: &LoadedResources,
    header: &str,
    question: &str,
) -> Result<String> {
    let output = ask_questions(
        state,
        resources,
        json!([{
            "type": "input",
            "header": header,
            "question": question
        }]),
    )?;
    answer_string(&output, question)
}

fn ask_searchable_choice(
    state: &mut AppState,
    resources: &LoadedResources,
    header: &str,
    question: &str,
    options: &[(String, String)],
) -> Result<String> {
    let options = options
        .iter()
        .map(|(label, description)| {
            json!({
                "label": label,
                "description": description
            })
        })
        .collect::<Vec<_>>();
    let output = ask_questions(
        state,
        resources,
        json!([{
            "type": "choice",
            "header": header,
            "question": question,
            "searchable": true,
            "options": options
        }]),
    )?;
    answer_string(&output, question)
}

fn ask_choice(
    state: &mut AppState,
    resources: &LoadedResources,
    header: &str,
    question: &str,
    options: &[(&str, &str)],
) -> Result<String> {
    ask_choice_with_preview(
        state,
        resources,
        header,
        question,
        &options
            .iter()
            .map(|(label, description)| (*label, *description, None))
            .collect::<Vec<_>>(),
    )
}

fn ask_choice_with_preview(
    state: &mut AppState,
    resources: &LoadedResources,
    header: &str,
    question: &str,
    options: &[(&str, &str, Option<&str>)],
) -> Result<String> {
    let options = options
        .iter()
        .map(|(label, description, preview)| {
            let mut option = json!({
                "label": label,
                "description": description
            });
            if let Some(preview) = preview {
                option["preview"] = Value::String((*preview).to_string());
            }
            option
        })
        .collect::<Vec<_>>();
    let output = ask_questions(
        state,
        resources,
        json!([{
            "type": "choice",
            "header": header,
            "question": question,
            "options": options
        }]),
    )?;
    answer_string(&output, question)
}

fn ask_questions(
    state: &mut AppState,
    resources: &LoadedResources,
    questions: Value,
) -> Result<Value> {
    let output = call_tool(
        state,
        resources,
        "AskUserQuestion",
        json!({ "questions": questions }),
    )?;
    if output
        .get("pending")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        bail!("interactive AskUserQuestion response is required for /connect");
    }
    Ok(output)
}

fn answer_string(output: &Value, question: &str) -> Result<String> {
    let value = output
        .get("answers")
        .and_then(|answers| answers.get(question))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("no answer provided for `{question}`"))?;
    Ok(value.to_string())
}

fn call_tool(
    state: &mut AppState,
    resources: &LoadedResources,
    tool_id: &str,
    input: Value,
) -> Result<Value> {
    let cwd = state.cwd.clone();
    let output = execute_workflow_tool(state, resources, &cwd, tool_id, input, None)?;
    serde_json::from_str(&output).or_else(|_| Ok(json!({ "status": "complete", "output": output })))
}

fn summary(connector_slug: &str, connection: &str, method: &str, output: &Value) -> ConnectResult {
    let status = status(output).unwrap_or("complete");
    let registered = output
        .get("registered_connection")
        .or_else(|| output.pointer("/connection/registered_connection"))
        .and_then(Value::as_bool);
    let registered_text = registered
        .map(|value| if value { "created" } else { "already existed" })
        .unwrap_or("not reported");
    ConnectResult {
        summary: format!(
            "Connector connection configured.\nconnector: {connector_slug}\nconnection: {connection}\nmethod: {method}\nstatus: {status}\nconnection record: {registered_text}"
        ),
    }
}

fn status(output: &Value) -> Option<&str> {
    output.get("status").and_then(Value::as_str)
}

#[cfg(test)]
#[path = "connect_tests.rs"]
mod tests;
