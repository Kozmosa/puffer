use crate::command_helpers::{
    append_tool_invocations, copy_last_message, describe_context, describe_git_diff,
    describe_permissions, describe_plugin, emit_system, execute_skill_command,
    handle_agents_command, handle_config_command, handle_hooks_command,
    handle_keybindings_command, handle_memory_command, list_ides, list_mcp_servers, list_skills,
    rewind_transcript, run_doctor, terminal_setup_advice,
};
use crate::{AppState, MessageRole};
use anyhow::Result;
use puffer_provider_openai::{
    build_authorization_url as build_openai_authorization_url,
    generate_pkce as generate_openai_pkce, OpenAIOAuthConfig,
};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::{prompt_by_id, LoadedResources};
use puffer_session_store::{SessionStore, TranscriptEvent};
use puffer_transport_anthropic::{
    build_authorization_url as build_anthropic_authorization_url,
    generate_pkce as generate_anthropic_pkce, AnthropicOAuthConfig,
};
use serde::Serialize;
use std::fmt::Write as _;
use std::path::PathBuf;

/// Distinguishes prompt-backed, local, and UI-oriented slash commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CommandKind {
    Prompt,
    Local,
    Ui,
}

/// Declares one built-in slash command supported by Puffer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub argument_hint: Option<&'static str>,
    pub kind: CommandKind,
}

/// Returns the full built-in slash-command surface supported by Puffer.
pub fn supported_commands() -> Vec<CommandSpec> {
    vec![
        cmd(
            "add-dir",
            &[],
            "Add a new working directory",
            Some("<path>"),
            CommandKind::Local,
        ),
        cmd(
            "agents",
            &[],
            "Manage agent configurations",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "branch",
            &["fork"],
            "Create a branch of the current conversation at this point",
            Some("[name]"),
            CommandKind::Local,
        ),
        cmd(
            "btw",
            &[],
            "Capture a quick side question or context note",
            Some("<question>"),
            CommandKind::Prompt,
        ),
        cmd(
            "buddy",
            &[],
            "Show or interact with Clawd",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "clear",
            &["reset", "new"],
            "Clear conversation history and free up context",
            None,
            CommandKind::Local,
        ),
        cmd(
            "color",
            &[],
            "Set the prompt bar color for this session",
            Some("<color|default>"),
            CommandKind::Ui,
        ),
        cmd(
            "compact",
            &[],
            "Summarize the conversation to preserve context budget",
            Some("<optional custom summarization instructions>"),
            CommandKind::Prompt,
        ),
        cmd(
            "config",
            &["settings"],
            "Open config panel",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "context",
            &[],
            "Show current context usage",
            None,
            CommandKind::Local,
        ),
        cmd(
            "copy",
            &[],
            "Copy the latest assistant output",
            None,
            CommandKind::Local,
        ),
        cmd(
            "cost",
            &[],
            "Show the total cost and duration of the current session",
            None,
            CommandKind::Local,
        ),
        cmd(
            "diff",
            &[],
            "View uncommitted changes and per-turn diffs",
            None,
            CommandKind::Local,
        ),
        cmd(
            "doctor",
            &[],
            "Diagnose and verify your Puffer Code installation and settings",
            None,
            CommandKind::Local,
        ),
        cmd(
            "effort",
            &[],
            "Set effort level for model usage",
            Some("[low|medium|high|max|auto]"),
            CommandKind::Ui,
        ),
        cmd("exit", &["quit"], "Exit the REPL", None, CommandKind::Local),
        cmd(
            "export",
            &[],
            "Export the current conversation to a file or clipboard",
            Some("[filename]"),
            CommandKind::Local,
        ),
        cmd(
            "fast",
            &[],
            "Toggle fast mode",
            Some("[on|off]"),
            CommandKind::Ui,
        ),
        cmd(
            "help",
            &[],
            "Show help and available commands",
            None,
            CommandKind::Local,
        ),
        cmd(
            "hooks",
            &[],
            "View hook configurations for tool events",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "ide",
            &[],
            "Manage IDE integrations and show status",
            Some("[open]"),
            CommandKind::Ui,
        ),
        cmd(
            "init",
            &[],
            "Initialize project guidance and defaults",
            None,
            CommandKind::Prompt,
        ),
        cmd(
            "keybindings",
            &[],
            "Open or create your keybindings configuration file",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "login",
            &[],
            "Sign in to a provider",
            Some("[provider]"),
            CommandKind::Ui,
        ),
        cmd(
            "logout",
            &[],
            "Sign out from a provider",
            Some("[provider]"),
            CommandKind::Ui,
        ),
        cmd(
            "mcp",
            &[],
            "Manage MCP servers",
            Some("[enable|disable [server-name]]"),
            CommandKind::Ui,
        ),
        cmd("memory", &[], "Edit memory files", None, CommandKind::Ui),
        cmd(
            "model",
            &[],
            "Select the active model",
            Some("[provider/model]"),
            CommandKind::Ui,
        ),
        cmd(
            "permissions",
            &["allowed-tools"],
            "Manage allow & deny tool permission rules",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "plan",
            &[],
            "Enable plan mode or view the current session plan",
            Some("[open|<description>]"),
            CommandKind::Prompt,
        ),
        cmd(
            "plugin",
            &["plugins", "marketplace"],
            "Manage Puffer plugins",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "pr-comments",
            &[],
            "Get comments from a GitHub pull request",
            None,
            CommandKind::Prompt,
        ),
        cmd(
            "reload-plugins",
            &[],
            "Activate pending plugin changes in the current session",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "remote-control",
            &["rc"],
            "Connect this terminal for remote-control sessions",
            Some("[name]"),
            CommandKind::Ui,
        ),
        cmd(
            "remote-env",
            &[],
            "Configure the default remote environment for remote sessions",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "rename",
            &[],
            "Rename the current conversation",
            Some("[name]"),
            CommandKind::Local,
        ),
        cmd(
            "resume",
            &["continue"],
            "Resume a previous conversation",
            Some("[conversation id or search term]"),
            CommandKind::Ui,
        ),
        cmd(
            "rewind",
            &["checkpoint"],
            "Restore the code and/or conversation to a previous point",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "review",
            &[],
            "Review the current worktree or pull request",
            None,
            CommandKind::Prompt,
        ),
        cmd(
            "sandbox",
            &[],
            "Manage sandbox policies",
            Some("exclude \"command pattern\""),
            CommandKind::Ui,
        ),
        cmd(
            "security-review",
            &[],
            "Complete a security review of the pending changes on the current branch",
            None,
            CommandKind::Prompt,
        ),
        cmd(
            "session",
            &["remote"],
            "Show remote session URL and QR code",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "skills",
            &[],
            "List available skills",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "status",
            &[],
            "Show current session configuration and status",
            None,
            CommandKind::Local,
        ),
        cmd(
            "statusline",
            &[],
            "Set up the status line UI",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "tasks",
            &["bashes"],
            "List and manage background tasks",
            None,
            CommandKind::Ui,
        ),
        cmd(
            "terminal-setup",
            &[],
            "Show terminal setup guidance",
            None,
            CommandKind::Local,
        ),
        cmd("theme", &[], "Change the theme", None, CommandKind::Local),
        cmd(
            "usage",
            &[],
            "Show plan usage limits",
            None,
            CommandKind::Local,
        ),
        cmd(
            "vim",
            &[],
            "Toggle between Vim and Normal editing modes",
            None,
            CommandKind::Local,
        ),
    ]
}

/// Finds a command by canonical name or alias.
pub fn find_command<'a>(commands: &'a [CommandSpec], name: &str) -> Option<&'a CommandSpec> {
    commands
        .iter()
        .find(|command| command.name == name || command.aliases.iter().any(|alias| *alias == name))
}

/// Dispatches a slash-command against the current application state.
pub fn dispatch_command(
    state: &mut AppState,
    commands: &[CommandSpec],
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    session_store: &SessionStore,
    raw_input: &str,
) -> Result<()> {
    let trimmed = raw_input.trim();
    let without_slash = trimmed.strip_prefix('/').unwrap_or(trimmed);
    let (name, args) = without_slash
        .split_once(' ')
        .map(|(name, args)| (name, args.trim()))
        .unwrap_or((without_slash, ""));

    if let Some(skill_name) = name.strip_prefix("skill:") {
        return execute_skill_command(state, resources, session_store, skill_name);
    }

    let Some(command) = find_command(commands, name) else {
        return emit_system(state, session_store, format!("Unknown command: /{name}"));
    };

    session_store.append_event(
        state.session.id,
        TranscriptEvent::CommandInvoked {
            name: command.name.to_string(),
            args: args.to_string(),
        },
    )?;

    match command.kind {
        CommandKind::Prompt => execute_prompt_command(
            state,
            resources,
            providers,
            auth_store,
            session_store,
            command,
            args,
        ),
        CommandKind::Local | CommandKind::Ui => {
            execute_local_command(
                state,
                commands,
                resources,
                providers,
                auth_store,
                session_store,
                command,
                args,
            )?;
            session_store.append_event(state.session.id, state.snapshot_event())
        }
    }
}

fn execute_prompt_command(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    session_store: &SessionStore,
    command: &CommandSpec,
    args: &str,
) -> Result<()> {
    let rendered = prompt_by_id(resources, command.name)
        .map(|prompt| prompt.value.template.replace("$ARGUMENTS", args))
        .unwrap_or_else(|| format!("Prompt command /{} invoked with: {}", command.name, args));
    state.push_message(MessageRole::User, rendered.clone());
    session_store.append_event(
        state.session.id,
        TranscriptEvent::UserMessage {
            text: rendered.clone(),
        },
    )?;

    match crate::runtime::execute_user_prompt(state, resources, providers, auth_store, &rendered) {
        Ok(turn) => {
            append_tool_invocations(state, session_store, &turn.tool_invocations)?;
            state.push_message(MessageRole::Assistant, turn.assistant_text.clone());
            session_store.append_event(
                state.session.id,
                TranscriptEvent::AssistantMessage {
                    text: turn.assistant_text,
                },
            )?;
        }
        Err(error) => {
            let message = format!("Prompt command /{} failed: {error}", command.name);
            emit_system(state, session_store, message)?;
        }
    }

    Ok(())
}

fn execute_local_command(
    state: &mut AppState,
    commands: &[CommandSpec],
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    session_store: &SessionStore,
    command: &CommandSpec,
    args: &str,
) -> Result<()> {
    match command.name {
        "help" => {
            let mut text = String::from("Supported commands:\n");
            for command in commands {
                let _ = writeln!(&mut text, "/{:<16} {}", command.name, command.description);
            }
            let _ = writeln!(
                &mut text,
                "\nResource counts: prompts={} tools={} skills={} plugins={} mcp_servers={} ides={}",
                resources.prompts.len(),
                resources.tools.len(),
                resources.skills.len(),
                resources.plugins.len(),
                resources.mcp_servers.len(),
                resources.ides.len()
            );
            emit_system(state, session_store, text)
        }
        "status" => emit_system(
            state,
            session_store,
            format!(
                "provider={:?} model={:?} theme={} color={} effort={} fast={} sandbox={} vim={} messages={} slug={} parent={:?}",
                state.current_provider,
                state.current_model,
                state.config.theme,
                state.prompt_color,
                state.effort_level,
                state.fast_mode,
                state.sandbox_mode,
                state.vim_mode,
                state.transcript.len(),
                state.session.slug.as_deref().unwrap_or("<none>"),
                state.session.parent_session_id
            ),
        ),
        "usage" => emit_system(
            state,
            session_store,
            format!(
                "messages={} commands={} providers={} skills={} plugins={}",
                state.transcript.len(),
                commands.len(),
                providers.providers().count(),
                resources.skills.len(),
                resources.plugins.len()
            ),
        ),
        "cost" => emit_system(state, session_store, render_cost_summary(state)),
        "clear" => {
            state.transcript.clear();
            emit_system(state, session_store, "Transcript cleared.".to_string())
        }
        "add-dir" => {
            let dir = if args.is_empty() { state.cwd.clone() } else { PathBuf::from(args) };
            if !state.working_dirs.iter().any(|existing| existing == &dir) {
                state.working_dirs.push(dir.clone());
            }
            emit_system(state, session_store, format!("Added working directory {}.", dir.display()))
        }
        "exit" => {
            state.should_exit = true;
            emit_system(state, session_store, "Exiting Puffer Code.".to_string())
        }
        "color" => {
            if args.is_empty() {
                emit_system(state, session_store, format!("Current prompt color is {}.", state.prompt_color))
            } else {
                state.prompt_color = args.to_string();
                emit_system(state, session_store, format!("Prompt color set to {}.", state.prompt_color))
            }
        }
        "effort" => {
            if args.is_empty() {
                emit_system(state, session_store, format!("Current effort level is {}.", state.effort_level))
            } else {
                state.effort_level = args.to_string();
                emit_system(state, session_store, format!("Effort level set to {}.", state.effort_level))
            }
        }
        "fast" => {
            if args.is_empty() {
                state.fast_mode = !state.fast_mode;
            } else {
                state.fast_mode = matches!(args, "on" | "true" | "1");
            }
            emit_system(
                state,
                session_store,
                format!("Fast mode is now {}.", if state.fast_mode { "on" } else { "off" }),
            )
        }
        "theme" => {
            if args.is_empty() {
                emit_system(state, session_store, format!("Current theme is {}.", state.config.theme))
            } else {
                state.config.theme = args.to_string();
                emit_system(state, session_store, format!("Theme set to {}.", state.config.theme))
            }
        }
        "vim" => {
            state.vim_mode = !state.vim_mode;
            emit_system(
                state,
                session_store,
                format!("Vim mode is now {}.", if state.vim_mode { "on" } else { "off" }),
            )
        }
        "model" => {
            if args.is_empty() {
                let models = providers
                    .models()
                    .map(|model| format!("{}/{}", model.provider, model.id))
                    .collect::<Vec<_>>()
                    .join(", ");
                emit_system(state, session_store, format!("Available models: {models}"))
            } else if let Some(model) = providers.resolve_model(args) {
                state.current_provider = Some(model.provider.clone());
                state.current_model = Some(format!("{}/{}", model.provider, model.id));
                emit_system(state, session_store, format!("Active model set to {}/{}.", model.provider, model.id))
            } else {
                emit_system(state, session_store, format!("Unknown model {args}."))
            }
        }
        "permissions" => describe_permissions(state, resources, session_store),
        "hooks" => handle_hooks_command(state, session_store, args),
        "statusline" => {
            if args.is_empty() {
                state.statusline_enabled = !state.statusline_enabled;
            } else {
                state.statusline_enabled = matches!(args, "on" | "true" | "1");
            }
            emit_system(
                state,
                session_store,
                format!(
                    "Status line is now {}.",
                    if state.statusline_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ),
            )
        }
        "tasks" => emit_system(
            state,
            session_store,
            render_task_summary(state),
        ),
        "config" => handle_config_command(state, session_store, args),
        "context" => describe_context(state, resources, session_store),
        "agents" => handle_agents_command(state, session_store, args),
        "memory" => handle_memory_command(state, session_store, args),
        "keybindings" => handle_keybindings_command(state, session_store),
        "remote-control" => {
            if args.is_empty() {
                emit_system(
                    state,
                    session_store,
                    format!(
                        "Remote control session name: {}",
                        state.remote_name.as_deref().unwrap_or("<unset>")
                    ),
                )
            } else {
                state.remote_name = Some(args.to_string());
                emit_system(
                    state,
                    session_store,
                    format!("Remote control session name set to {}.", args),
                )
            }
        }
        "remote-env" => {
            if args.is_empty() {
                emit_system(
                    state,
                    session_store,
                    format!(
                        "Remote environment: {}",
                        state.remote_environment.as_deref().unwrap_or("<unset>")
                    ),
                )
            } else {
                state.remote_environment = Some(args.to_string());
                emit_system(
                    state,
                    session_store,
                    format!("Remote environment set to {}.", args),
                )
            }
        }
        "sandbox" => {
            if args.is_empty() {
                emit_system(state, session_store, format!("Current sandbox mode is {}.", state.sandbox_mode))
            } else {
                state.sandbox_mode = args.to_string();
                emit_system(state, session_store, format!("Sandbox mode set to {}.", state.sandbox_mode))
            }
        }
        "rename" => {
            let name = if args.is_empty() { format!("session-{}", &state.session.id.to_string()[..8]) } else { args.to_string() };
            session_store.rename_session(state.session.id, name.clone())?;
            state.session.display_name = Some(name.clone());
            emit_system(state, session_store, format!("Session renamed to {name}."))
        }
        "export" => {
            let target = if args.is_empty() { state.cwd.join(format!("{}.json", state.session.id)) } else { PathBuf::from(args) };
            let payload = serde_json::to_string_pretty(&state.transcript)?;
            std::fs::write(&target, payload)?;
            emit_system(state, session_store, format!("Exported transcript to {}.", target.display()))
        }
        "copy" => copy_last_message(state, session_store),
        "diff" => describe_git_diff(state, session_store),
        "doctor" => run_doctor(state, resources, providers, session_store),
        "buddy" => emit_system(
            state,
            session_store,
            format!(
                "{} is on duty. Mascot id={}, enabled={}.",
                state.config.mascot.display_name,
                state.config.mascot.id,
                state.config.mascot.enabled
            ),
        ),
        "skills" => list_skills(state, resources, session_store),
        "plugin" => describe_plugin(state, resources, session_store, args),
        "reload-plugins" => emit_system(
            state,
            session_store,
            format!(
                "Reloaded plugin registry for this session.\nplugins={}\nskills={}\nmcp_servers={}",
                resources.plugins.len(),
                resources.skills.len(),
                resources.mcp_servers.len()
            ),
        ),
        "mcp" => list_mcp_servers(state, resources, session_store),
        "ide" => list_ides(state, resources, session_store),
        "login" => {
            let provider = if args.is_empty() {
                state.current_provider.as_deref().unwrap_or("anthropic")
            } else {
                args
            };
            let status = if auth_store.has_auth(provider) {
                "Credentials are already stored."
            } else {
                "No credentials are currently stored."
            };
            let oauth_hint = match provider {
                "openai" => {
                    let pkce = generate_openai_pkce();
                    let config = OpenAIOAuthConfig {
                        state: pkce.state.clone(),
                        code_challenge: pkce.challenge.clone(),
                        ..OpenAIOAuthConfig::default()
                    };
                    format!(
                        "OAuth start bundle:\nurl={}\nverifier={}\nstate={}",
                        build_openai_authorization_url(&config),
                        pkce.verifier,
                        pkce.state
                    )
                }
                "anthropic" => {
                    let pkce = generate_anthropic_pkce();
                    let config = AnthropicOAuthConfig {
                        state: pkce.state.clone(),
                        code_challenge: pkce.challenge.clone(),
                        ..AnthropicOAuthConfig::default()
                    };
                    format!(
                        "OAuth start bundle:\nurl={}\nverifier={}\nstate={}",
                        build_anthropic_authorization_url(&config),
                        pkce.verifier,
                        pkce.state
                    )
                }
                _ => format!("No built-in OAuth URL generator for {provider}."),
            };
            emit_system(
                state,
                session_store,
                format!(
                    "{status}\nRun `puffer auth login {provider}` for a guided OAuth flow or `puffer auth set-api-key {provider} --stdin` for API-key auth.\n{oauth_hint}"
                ),
            )
        }
        "logout" => {
            let provider = if args.is_empty() { "anthropic" } else { args };
            emit_system(
                state,
                session_store,
                format!(
                    "Run `puffer auth clear {provider}` to remove stored credentials for {provider}."
                ),
            )
        }
        "session" => emit_system(
            state,
            session_store,
            format!(
                "session_id={}\ncwd={}\ndisplay_name={}\nslug={}\nparent={:?}\ntags={}\nnote={}",
                state.session.id,
                state.session.cwd.display(),
                state.session.display_name.as_deref().unwrap_or("<unnamed>"),
                state.session.slug.as_deref().unwrap_or("<none>"),
                state.session.parent_session_id,
                if state.session.tags.is_empty() {
                    "<none>".to_string()
                } else {
                    state.session.tags.join(", ")
                },
                state.session.note.as_deref().unwrap_or("<none>")
            ),
        ),
        "resume" => {
            if args.is_empty() {
                let sessions = session_store.list_sessions()?;
                let mut text = String::from("Sessions:\n");
                for session in sessions.iter().take(10) {
                    let name = session.display_name.as_deref().unwrap_or("<unnamed>");
                    let _ = writeln!(&mut text, "{} {}", session.id, name);
                }
                emit_system(state, session_store, text)
            } else if let Some(session) = session_store.find_session(args)? {
                let record = session_store.load_session(session.id)?;
                let config = state.config.clone();
                *state = AppState::from_session_record(config, record);
                emit_system(
                    state,
                    session_store,
                    format!("Resumed session {}.", state.session.id),
                )
            } else {
                emit_system(state, session_store, format!("No session matched {args}."))
            }
        }
        "branch" => {
            let fork = session_store.fork_session(state.session.id, state.cwd.clone())?;
            if !args.is_empty() {
                session_store.rename_session(fork.id, args.to_string())?;
            }
            let record = session_store.load_session(fork.id)?;
            let config = state.config.clone();
            *state = AppState::from_session_record(config, record);
            if !args.is_empty() {
                state.session.display_name = Some(args.to_string());
            }
            emit_system(
                state,
                session_store,
                format!("Forked current session into {}.", state.session.id),
            )
        }
        "rewind" => rewind_transcript(state, session_store),
        "terminal-setup" => emit_system(state, session_store, terminal_setup_advice(state)),
        _ => emit_system(
            state,
            session_store,
            format!(
                "/{} is registered as a {:?} command but is not fully implemented yet.",
                command.name, command.kind
            ),
        ),
    }
}

fn cmd(
    name: &'static str,
    aliases: &'static [&'static str],
    description: &'static str,
    argument_hint: Option<&'static str>,
    kind: CommandKind,
) -> CommandSpec {
    CommandSpec {
        name,
        aliases,
        description,
        argument_hint,
        kind,
    }
}

fn render_task_summary(state: &AppState) -> String {
    if state.tasks().is_empty() {
        return "Tasks:\nNo recorded shell or tool tasks yet.".to_string();
    }

    let mut text = String::from("Tasks:\n");
    for task in state.tasks().iter().rev().take(10) {
        let status = match task.status {
            crate::TaskStatus::Completed => "completed",
            crate::TaskStatus::Failed => "failed",
        };
        let _ = writeln!(
            &mut text,
            "#{} {} [{}]\n{}",
            task.id, task.label, status, task.detail
        );
    }
    text.trim_end().to_string()
}

fn render_cost_summary(state: &AppState) -> String {
    let elapsed_ms = now_ms().saturating_sub(state.session.created_at_ms);
    let assistant_messages = state
        .transcript
        .iter()
        .filter(|message| message.role == MessageRole::Assistant)
        .count();
    let user_messages = state
        .transcript
        .iter()
        .filter(|message| message.role == MessageRole::User)
        .count();
    let tool_invocations = state
        .transcript
        .iter()
        .filter(|message| {
            message.role == MessageRole::System && message.text.starts_with("Tool ")
        })
        .count();
    format!(
        "Session cost summary:\nelapsed_ms={elapsed_ms}\nuser_messages={user_messages}\nassistant_messages={assistant_messages}\ntool_invocations={tool_invocations}\nrecorded_tasks={}\nestimated_cost_usd=unavailable",
        state.tasks().len()
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests;
