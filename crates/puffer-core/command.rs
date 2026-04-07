use crate::command_helpers::{
    append_tool_invocations, describe_context, describe_files_in_context, describe_git_diff,
    emit_system, execute_skill_command, handle_agents_command, handle_config_command,
    handle_copy_command, handle_export_command, handle_hooks_command, handle_ide_command,
    handle_keybindings_command, handle_mcp_command, handle_memory_command,
    handle_model_command, handle_permissions_command, handle_plugin_command,
    handle_remote_control_command, handle_remote_env_command, handle_resume_command,
    handle_sandbox_command, handle_session_command, handle_tag_command, handle_tasks_command,
    list_skills, record_command_checkpoint, reload_config_from_disk, render_login_guidance,
    rewind_transcript, run_doctor, terminal_setup_advice,
};
use crate::{
    render_buddy_summary, render_cost_summary, render_status_summary, render_usage_summary,
    AppState, MessageRole,
};
use anyhow::Result;
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::{prompt_by_id, render_prompt_by_id, LoadedResources};
use puffer_session_store::{SessionStore, TranscriptEvent};
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
            "Ask a quick side question without interrupting the main conversation",
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
            "files",
            &[],
            "List all files currently in context",
            None,
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
            "Configure the remote environment for this session",
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
            "skill:<name>",
            &[],
            "Run a loaded skill by slash-safe skill name",
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
            CommandKind::Prompt,
        ),
        cmd(
            "tag",
            &[],
            "Toggle a searchable tag on the current session",
            Some("<tag-name>"),
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
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
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
        session_store.append_event(
            state.session.id,
            TranscriptEvent::CommandInvoked {
                name: format!("skill:{skill_name}"),
                args: String::new(),
            },
        )?;
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
        CommandKind::Prompt => {
            execute_prompt_command(
                state,
                resources,
                providers,
                auth_store,
                session_store,
                command,
                args,
            )?;
            record_command_checkpoint(state, session_store, command.name, args)
        }
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
            record_command_checkpoint(state, session_store, command.name, args)
        }
    }
}

fn execute_prompt_command(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &mut AuthStore,
    session_store: &SessionStore,
    command: &CommandSpec,
    args: &str,
) -> Result<()> {
    let preparation = crate::command_helpers::prompt::prepare_prompt_command_specialization(
        state,
        session_store,
        command.name,
        args,
    )?;
    if matches!(
        preparation,
        Some(crate::command_helpers::prompt::PromptCommandPreparation::HandledLocally)
    ) {
        return Ok(());
    }

    if let Some(crate::command_helpers::prompt::PromptCommandPreparation::SideQuestion(question)) =
        preparation.clone()
    {
        match crate::runtime::execute_side_question(
            state, resources, providers, auth_store, &question,
        ) {
            Ok(turn) => {
                state.push_message(MessageRole::Assistant, turn.assistant_text.clone());
                session_store.append_event(
                    state.session.id,
                    TranscriptEvent::AssistantMessage {
                        text: turn.assistant_text,
                    },
                )?;
            }
            Err(error) => {
                emit_system(
                    state,
                    session_store,
                    format!("Prompt command /btw failed: {error}"),
                )?;
            }
        }
        return Ok(());
    }

    let prompt = prompt_by_id(resources, command.name);
    if let Some(prompt) = prompt {
        if let Some(model_override) = prompt
            .value
            .model_override
            .as_deref()
            .and_then(|model_id| providers.resolve_model(model_id))
        {
            state.current_provider = Some(model_override.provider.clone());
            state.current_model =
                Some(format!("{}/{}", model_override.provider, model_override.id));
        } else if let Some(provider_override) = prompt.value.provider_override.as_ref() {
            state.current_provider = Some(provider_override.clone());
        }
    }
    let mut variables = std::collections::BTreeMap::from([
        ("ARGUMENTS".to_string(), args.to_string()),
        ("CWD".to_string(), state.cwd.display().to_string()),
        (
            "PROVIDER".to_string(),
            state.current_provider.clone().unwrap_or_default(),
        ),
        (
            "MODEL".to_string(),
            state.current_model.clone().unwrap_or_default(),
        ),
        ("SESSION_ID".to_string(), state.session.id.to_string()),
        (
            "SESSION_SLUG".to_string(),
            state.session.slug.clone().unwrap_or_default(),
        ),
    ]);
    if let Some(crate::command_helpers::prompt::PromptCommandPreparation::VariableOverrides(
        extra_variables,
    )) = preparation.as_ref()
    {
        variables.extend(extra_variables.clone());
    }
    let mut rendered = match preparation {
        Some(crate::command_helpers::prompt::PromptCommandPreparation::PromptOverride(prompt)) => {
            prompt
        }
        Some(crate::command_helpers::prompt::PromptCommandPreparation::SideQuestion(_)) => {
            unreachable!("handled above")
        }
        Some(crate::command_helpers::prompt::PromptCommandPreparation::VariableOverrides(_))
        | None => render_prompt_by_id(resources, command.name, &variables)
            .unwrap_or_else(|| format!("Prompt command /{} invoked with: {}", command.name, args)),
        Some(crate::command_helpers::prompt::PromptCommandPreparation::HandledLocally) => {
            unreachable!("handled above")
        }
    };
    if let Some(mode) = prompt.and_then(|prompt| prompt.value.mode.as_deref()) {
        rendered = format!("Command mode: {mode}\n\n{rendered}");
    }

    if command.name == "compact" {
        return crate::command_helpers::prompt::execute_compact_prompt_command(
            state,
            resources,
            providers,
            auth_store,
            session_store,
            &rendered,
        );
    }

    if command.name == "plan" && !args.trim().is_empty() && !matches!(args.trim(), "show" | "open")
    {
        return crate::command_helpers::prompt::execute_plan_prompt_command(
            state,
            resources,
            providers,
            auth_store,
            session_store,
            &rendered,
        );
    }

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
            if command.name == "statusline" {
                let _ = reload_config_from_disk(state);
            }
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
    providers: &mut ProviderRegistry,
    auth_store: &mut AuthStore,
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
                "\nRun /skills to list loaded /skill:<name> entries."
            );
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
            render_status_summary(state, resources, providers, auth_store),
        ),
        "usage" => emit_system(
            state,
            session_store,
            render_usage_summary(state, commands, resources, providers, auth_store),
        ),
        "cost" => emit_system(state, session_store, render_cost_summary(state)),
        "clear" => {
            session_store.append_transcript_clear(state.session.id)?;
            state.apply_transcript_rewrite(&puffer_session_store::TranscriptRewrite::Clear);
            emit_system(state, session_store, "Transcript cleared.".to_string())
        }
        "add-dir" => {
            let dir = if args.is_empty() {
                state.cwd.clone()
            } else {
                PathBuf::from(args)
            };
            if !state.working_dirs.iter().any(|existing| existing == &dir) {
                state.working_dirs.push(dir.clone());
            }
            emit_system(
                state,
                session_store,
                format!("Added working directory {}.", dir.display()),
            )
        }
        "exit" => {
            state.should_exit = true;
            emit_system(state, session_store, "Exiting Puffer Code.".to_string())
        }
        "color" => {
            if args.is_empty() {
                emit_system(
                    state,
                    session_store,
                    format!("Current prompt color is {}.", state.prompt_color),
                )
            } else {
                state.prompt_color = args.to_string();
                emit_system(
                    state,
                    session_store,
                    format!("Prompt color set to {}.", state.prompt_color),
                )
            }
        }
        "effort" => {
            if args.is_empty() {
                emit_system(
                    state,
                    session_store,
                    format!("Current effort level is {}.", state.effort_level),
                )
            } else {
                state.effort_level = args.to_string();
                emit_system(
                    state,
                    session_store,
                    format!("Effort level set to {}.", state.effort_level),
                )
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
                format!(
                    "Fast mode is now {}.",
                    if state.fast_mode { "on" } else { "off" }
                ),
            )
        }
        "theme" => {
            if args.is_empty() {
                emit_system(
                    state,
                    session_store,
                    format!("Current theme is {}.", state.config.theme),
                )
            } else {
                state.config.theme = args.to_string();
                emit_system(
                    state,
                    session_store,
                    format!("Theme set to {}.", state.config.theme),
                )
            }
        }
        "vim" => {
            state.vim_mode = !state.vim_mode;
            emit_system(
                state,
                session_store,
                format!(
                    "Vim mode is now {}.",
                    if state.vim_mode { "on" } else { "off" }
                ),
            )
        }
        "model" => handle_model_command(state, providers, auth_store, session_store, args),
        "permissions" => handle_permissions_command(state, resources, session_store, args),
        "hooks" => handle_hooks_command(state, resources, session_store, args),
        "tasks" => handle_tasks_command(state, session_store, args),
        "config" => handle_config_command(state, session_store, args),
        "context" => describe_context(state, resources, session_store),
        "agents" => handle_agents_command(state, session_store, args),
        "memory" => handle_memory_command(state, session_store, args),
        "keybindings" => handle_keybindings_command(state, session_store),
        "remote-control" => handle_remote_control_command(state, session_store, args),
        "remote-env" => handle_remote_env_command(state, session_store, args),
        "sandbox" => handle_sandbox_command(state, session_store, args),
        "rename" => {
            let name = if args.is_empty() {
                format!("session-{}", &state.session.id.to_string()[..8])
            } else {
                args.to_string()
            };
            session_store.rename_session(state.session.id, name.clone())?;
            state.session.display_name = Some(name.clone());
            emit_system(state, session_store, format!("Session renamed to {name}."))
        }
        "export" => handle_export_command(state, session_store, args),
        "copy" => handle_copy_command(state, session_store, args),
        "diff" => describe_git_diff(state, session_store),
        "files" => describe_files_in_context(state, session_store),
        "doctor" => run_doctor(state, resources, providers, auth_store, session_store),
        "buddy" => emit_system(state, session_store, render_buddy_summary(state, resources)),
        "skills" => list_skills(state, resources, session_store),
        "plugin" => handle_plugin_command(state, resources, session_store, args),
        "reload-plugins" => {
            state.reload_resources_requested = true;
            emit_system(
                state,
                session_store,
                "Reloading plugin changes from disk for this session...".to_string(),
            )
        }
        "mcp" => handle_mcp_command(state, resources, session_store, args),
        "ide" => handle_ide_command(state, resources, session_store, args),
        "login" => {
            let provider = if args.is_empty() {
                state.current_provider.as_deref().unwrap_or("anthropic")
            } else {
                args
            };
            let descriptor = providers.provider(provider);
            emit_system(
                state,
                session_store,
                render_login_guidance(provider, descriptor, auth_store.has_auth(provider)),
            )
        }
        "logout" => {
            let provider = if args.is_empty() { "anthropic" } else { args };
            if providers
                .provider(provider)
                .map(|descriptor| descriptor.auth_modes.is_empty())
                .unwrap_or(false)
            {
                return emit_system(
                    state,
                    session_store,
                    format!("{provider} does not use stored credentials."),
                );
            }
            emit_system(
                state,
                session_store,
                format!(
                    "Run `puffer auth clear {provider}` to remove stored credentials for {provider}."
                ),
            )
        }
        "session" => handle_session_command(state, session_store, args),
        "tag" => handle_tag_command(state, session_store, args),
        "resume" => handle_resume_command(state, session_store, args),
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
            state.remote_name = None;
            state.remote_session_id = None;
            state.remote_session_url = None;
            state.remote_session_status = None;
            emit_system(
                state,
                session_store,
                format!("Forked current session into {}.", state.session.id),
            )
        }
        "rewind" => rewind_transcript(state, session_store, args),
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

#[cfg(test)]
mod tests;
