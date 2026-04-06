mod runtime;

use anyhow::Result;
use arboard::Clipboard;
use puffer_config::PufferConfig;
use puffer_provider_openai::{
    build_authorization_url as build_openai_authorization_url,
    generate_pkce as generate_openai_pkce, OpenAIOAuthConfig,
};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use puffer_resources::{
    plugin_by_id, plugin_mcp_servers, prompt_by_id, skill_by_name, LoadedResources,
};
use puffer_session_store::{SessionMetadata, SessionStore, TranscriptEvent};
use puffer_tools::ToolRegistry;
use puffer_transport_anthropic::{
    build_authorization_url as build_anthropic_authorization_url,
    generate_pkce as generate_anthropic_pkce, AnthropicOAuthConfig,
};
use serde::Serialize;
use std::fmt::Write as _;
use std::process::Command;
use std::path::PathBuf;

pub use runtime::execute_user_prompt as execute_user_turn;

/// Describes the role of a rendered transcript message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

/// Represents one rendered transcript message in the interactive UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenderedMessage {
    pub role: MessageRole,
    pub text: String,
}

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

/// Stores the mutable session and UI state for one interactive Puffer run.
#[derive(Debug, Clone)]
pub struct AppState {
    pub config: PufferConfig,
    pub cwd: PathBuf,
    pub working_dirs: Vec<PathBuf>,
    pub session: SessionMetadata,
    pub transcript: Vec<RenderedMessage>,
    pub current_model: Option<String>,
    pub current_provider: Option<String>,
    pub prompt_color: String,
    pub effort_level: String,
    pub fast_mode: bool,
    pub sandbox_mode: String,
    pub remote_name: Option<String>,
    pub remote_environment: Option<String>,
    pub statusline_enabled: bool,
    pub vim_mode: bool,
    pub should_exit: bool,
}

impl AppState {
    /// Creates a new application state for the active session.
    pub fn new(config: PufferConfig, cwd: PathBuf, session: SessionMetadata) -> Self {
        Self {
            current_model: config.default_model.clone(),
            current_provider: config.default_provider.clone(),
            config,
            cwd,
            working_dirs: Vec::new(),
            session,
            transcript: Vec::new(),
            prompt_color: "default".to_string(),
            effort_level: "medium".to_string(),
            fast_mode: false,
            sandbox_mode: "workspace-write".to_string(),
            remote_name: None,
            remote_environment: None,
            statusline_enabled: true,
            vim_mode: false,
            should_exit: false,
        }
    }

    /// Appends a rendered message to the in-memory transcript.
    pub fn push_message(&mut self, role: MessageRole, text: impl Into<String>) {
        self.transcript.push(RenderedMessage {
            role,
            text: text.into(),
        });
    }
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
            Some("[plugin-id]"),
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
        CommandKind::Prompt => {
            execute_prompt_command(state, resources, session_store, command, args)
        }
        CommandKind::Local | CommandKind::Ui => execute_local_command(
            state,
            commands,
            resources,
            providers,
            auth_store,
            session_store,
            command,
            args,
        ),
    }
}

fn execute_prompt_command(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    command: &CommandSpec,
    args: &str,
) -> Result<()> {
    let rendered = prompt_by_id(resources, command.name)
        .map(|prompt| prompt.value.template.replace("$ARGUMENTS", args))
        .unwrap_or_else(|| format!("Prompt command /{} invoked with: {}", command.name, args));
    emit_system(state, session_store, rendered)
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
            let registry = ToolRegistry::from_resources(resources);
            let mut text = String::from("Supported commands:\n");
            for command in commands {
                let _ = writeln!(&mut text, "/{:<16} {}", command.name, command.description);
            }
            let _ = writeln!(
                &mut text,
                "\nResource counts: prompts={} tools={} executable_tools={} skills={} plugins={} mcp_servers={} ides={}",
                resources.prompts.len(),
                resources.tools.len(),
                registry.tools().count(),
                resources.skills.len(),
                resources.plugins.len(),
                resources.mcp_servers.len(),
                resources.ides.len()
            );
            emit_system(state, session_store, text)
        }
        "status" => {
            let registry = ToolRegistry::from_resources(resources);
            emit_system(
                state,
                session_store,
                format!(
                    "provider={:?} model={:?} theme={} color={} effort={} fast={} sandbox={} vim={} messages={} slug={} parent={:?} executable_tools={}",
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
                    state.session.parent_session_id,
                    registry.tools().count()
                ),
            )
        }
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
        "context" => emit_system(
            state,
            session_store,
            format!(
                "Context summary:\nmessages={}\ncharacters={}\nprompts={}\ntools={}\nskills={}",
                state.transcript.len(),
                state.transcript.iter().map(|message| message.text.len()).sum::<usize>(),
                resources.prompts.len(),
                resources.tools.len(),
                resources.skills.len()
            ),
        ),
        "clear" => {
            state.transcript.clear();
            emit_system(state, session_store, "Transcript cleared.".to_string())
        }
        "copy" => copy_last_message(state, session_store),
        "context" => describe_context(state, resources, session_store),
        "diff" => describe_git_diff(state, session_store),
        "doctor" => run_doctor(state, resources, providers, session_store),
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
                emit_system(
                    state,
                    session_store,
                    format!("Active model set to {}/{}.", model.provider, model.id),
                )
            } else {
                emit_system(state, session_store, format!("Unknown model {args}."))
            }
        }
        "copy" => {
            let copied = state
                .transcript
                .iter()
                .rev()
                .find(|message| matches!(message.role, MessageRole::Assistant))
                .map(|message| message.text.clone())
                .unwrap_or_else(|| "No assistant message is available to copy.".to_string());
            emit_system(
                state,
                session_store,
                format!("Latest assistant output:\n{copied}"),
            )
        }
        "cost" => emit_system(
            state,
            session_store,
            format!(
                "Cost summary:\nprovider={}\nmodel={}\ntracked_cost_usd=0.00\ntracked_turns={}",
                state.current_provider.as_deref().unwrap_or("<unset>"),
                state.current_model.as_deref().unwrap_or("<unset>"),
                state.transcript
                    .iter()
                    .filter(|message| matches!(message.role, MessageRole::Assistant))
                    .count()
            ),
        ),
        "diff" => emit_system(
            state,
            session_store,
            render_git_diff_summary(&state.cwd),
        ),
        "permissions" => describe_permissions(state, resources, session_store),
        "doctor" => run_doctor(state, resources, providers, session_store),
        "hooks" => emit_system(
            state,
            session_store,
            format!(
                "Hook configuration:\nexternal_handlers=0\nresource_tools={}\nresource_prompts={}",
                resources.tools.len(),
                resources.prompts.len()
            ),
        ),
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
            format!(
                "Background tasks:\nqueued=0\nrunning=0\ncompleted=0\nknown_tools={}",
                ToolRegistry::from_resources(resources).tools().count()
            ),
        ),
        "reload-plugins" => {
            let plugin_ids = resources
                .plugins
                .iter()
                .map(|plugin| plugin.value.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            emit_system(
                state,
                session_store,
                format!(
                    "Plugin registry refreshed in-memory.\nloaded_plugins={}",
                    if plugin_ids.is_empty() { "<none>" } else { &plugin_ids }
                ),
            )
        }
        "config" => emit_system(
            state,
            session_store,
            format!(
                "Config summary:\napp_name={}\ndefault_provider={}\ndefault_model={}\ntheme={}\nno_alt_screen={}\ntmux_golden_mode={}",
                state.config.app_name,
                state.config.default_provider.as_deref().unwrap_or("<unset>"),
                state.config.default_model.as_deref().unwrap_or("<unset>"),
                state.config.theme,
                state.config.ui.no_alt_screen,
                state.config.ui.tmux_golden_mode,
            ),
        ),
        "agents" => emit_system(
            state,
            session_store,
            "Agent management:\ninteractive_agent=default\nforkable_sessions=yes\nnamed_presets=not yet implemented".to_string(),
        ),
        "memory" => emit_system(
            state,
            session_store,
            format!(
                "Session memory summary:\nslug={}\nnote={}\ntags={}",
                state.session.slug.as_deref().unwrap_or("<none>"),
                state.session.note.as_deref().unwrap_or("<none>"),
                if state.session.tags.is_empty() {
                    "<none>".to_string()
                } else {
                    state.session.tags.join(", ")
                },
            ),
        ),
        "keybindings" => emit_system(
            state,
            session_store,
            "Keybinding defaults:\nenter=submit\nesc=clear input\nctrl+c=exit".to_string(),
        ),
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
                emit_system(
                    state,
                    session_store,
                    format!("Current sandbox mode is {}.", state.sandbox_mode),
                )
            } else {
                state.sandbox_mode = args.to_string();
                emit_system(
                    state,
                    session_store,
                    format!("Sandbox mode set to {}.", state.sandbox_mode),
                )
            }
        }
        "rewind" => rewind_transcript(state, session_store),
        "rename" => {
            let name = if args.is_empty() {
                format!("session-{}", &state.session.id.to_string()[..8])
            } else {
                args.to_string()
            };
            session_store.rename_session(state.session.id, name.clone())?;
            emit_system(state, session_store, format!("Session renamed to {name}."))
        }
        "export" => {
            let target = if args.is_empty() {
                state.cwd.join(format!("{}.json", state.session.id))
            } else {
                PathBuf::from(args)
            };
            let payload = serde_json::to_string_pretty(&state.transcript)?;
            std::fs::write(&target, payload)?;
            emit_system(
                state,
                session_store,
                format!("Exported transcript to {}.", target.display()),
            )
        }
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
            } else {
                let session_id = args
                    .parse()
                    .map_err(|_| anyhow::anyhow!("resume expects a session UUID"))?;
                let record = session_store.load_session(session_id)?;
                state.session = record.metadata;
                state.cwd = state.session.cwd.clone();
                state.transcript = record
                    .events
                    .iter()
                    .filter_map(event_to_rendered_message)
                    .collect();
                emit_system(
                    state,
                    session_store,
                    format!("Resumed session {}.", state.session.id),
                )
            }
        }
        "branch" => {
            let fork = session_store.fork_session(state.session.id, state.cwd.clone())?;
            emit_system(
                state,
                session_store,
                format!("Forked current session into {}.", fork.id),
            )
        }
        "terminal-setup" => emit_system(
            state,
            session_store,
            terminal_setup_advice(state),
        ),
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

fn list_skills(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    if resources.skills.is_empty() {
        return emit_system(state, session_store, "No skills are available.".to_string());
    }
    let mut text = String::from("Available skills:\n");
    for skill in &resources.skills {
        let _ = writeln!(
            &mut text,
            "/skill:{} - {}",
            skill.value.name, skill.value.description
        );
    }
    emit_system(state, session_store, text)
}

fn describe_permissions(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    let registry = ToolRegistry::from_resources(resources);
    if registry.tools().count() == 0 {
        return emit_system(
            state,
            session_store,
            "No tool metadata is loaded.".to_string(),
        );
    }

    let mut text = String::from("Tool permission summary:\n");
    for tool in registry.tools() {
        let _ = writeln!(
            &mut text,
            "- {} [{}]: approval={} sandbox={} executable=yes",
            tool.spec.name,
            tool.spec.handler,
            tool.spec
                .approval_policy
                .as_deref()
                .unwrap_or("<unspecified>"),
            tool.spec
                .sandbox_policy
                .as_deref()
                .unwrap_or("<unspecified>")
        );
    }
    emit_system(state, session_store, text)
}

fn render_git_diff_summary(cwd: &PathBuf) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .arg("diff")
        .arg("--stat")
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if text.is_empty() {
                "Git diff summary:\n<no changes>".to_string()
            } else {
                format!("Git diff summary:\n{text}")
            }
        }
        Ok(output) => format!(
            "Git diff summary unavailable:\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => format!("Git diff summary unavailable:\n{error}"),
    }
}

fn rewind_transcript(state: &mut AppState, session_store: &SessionStore) -> Result<()> {
    let index = state
        .transcript
        .iter()
        .rposition(|message| !matches!(message.role, MessageRole::System));
    match index {
        Some(index) => {
            let removed = state.transcript.remove(index);
            emit_system(
                state,
                session_store,
                format!("Rewound the last visible message: {}", removed.text),
            )
        }
        None => emit_system(
            state,
            session_store,
            "There is no visible message to rewind.".to_string(),
        ),
    }
}

fn terminal_setup_advice(state: &AppState) -> String {
    format!(
        "Terminal setup guidance:\nno_alt_screen={}\ntmux_golden_mode={}\nIf scrolling is awkward, try `puffer --no-alt-screen`.",
        state.config.ui.no_alt_screen,
        state.config.ui.tmux_golden_mode
    )
}

fn describe_plugin(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    args: &str,
) -> Result<()> {
    if args.is_empty() {
        if resources.plugins.is_empty() {
            return emit_system(
                state,
                session_store,
                "No plugins are installed.".to_string(),
            );
        }
        let mut text = String::from("Plugins:\n");
        for plugin in &resources.plugins {
            let _ = writeln!(
                &mut text,
                "{} - {}",
                plugin.value.id, plugin.value.description
            );
        }
        return emit_system(state, session_store, text);
    }

    let Some(plugin) = plugin_by_id(resources, args) else {
        return emit_system(state, session_store, format!("Unknown plugin {args}."));
    };
    let mut text = format!("Plugin {}\n{}\n", plugin.value.id, plugin.value.description);
    if !plugin.value.commands.is_empty() {
        let commands = plugin
            .value
            .commands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(&mut text, "Commands: {commands}");
    }
    if !plugin.value.skills.is_empty() {
        let _ = writeln!(&mut text, "Skills: {}", plugin.value.skills.join(", "));
    }
    if !plugin.value.mcp_servers.is_empty() {
        let ids = plugin
            .value
            .mcp_servers
            .iter()
            .map(|server| server.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(&mut text, "MCP servers: {ids}");
    }
    emit_system(state, session_store, text)
}

fn list_mcp_servers(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    let servers = plugin_mcp_servers(resources);
    if servers.is_empty() && resources.mcp_servers.is_empty() {
        return emit_system(
            state,
            session_store,
            "No MCP servers are configured.".to_string(),
        );
    }
    let mut text = String::from("MCP servers:\n");
    for server in &resources.mcp_servers {
        let target = if server.value.endpoint.is_empty() {
            server.value.target.as_str()
        } else {
            server.value.endpoint.as_str()
        };
        let _ = writeln!(
            &mut text,
            "{} [{}] -> {}",
            server.value.id, server.value.transport, target
        );
    }
    for (plugin, server) in servers {
        let target = if server.target.is_empty() {
            server.endpoint.as_str()
        } else {
            server.target.as_str()
        };
        let _ = writeln!(
            &mut text,
            "{}:{} [{}] -> {}",
            plugin.id, server.id, server.transport, target
        );
    }
    emit_system(state, session_store, text)
}

fn list_ides(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    if resources.ides.is_empty() {
        return emit_system(
            state,
            session_store,
            "No IDE integrations are configured.".to_string(),
        );
    }
    let mut text = String::from("IDE integrations:\n");
    for ide in &resources.ides {
        let _ = writeln!(
            &mut text,
            "{} - {}",
            ide.value.display_name, ide.value.description
        );
    }
    emit_system(state, session_store, text)
}

fn copy_last_message(state: &mut AppState, session_store: &SessionStore) -> Result<()> {
    let text = state
        .transcript
        .iter()
        .rev()
        .find(|message| matches!(message.role, MessageRole::Assistant | MessageRole::System))
        .map(|message| message.text.clone())
        .ok_or_else(|| anyhow::anyhow!("no assistant or system message is available to copy"))?;

    match Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text.clone())) {
        Ok(()) => emit_system(
            state,
            session_store,
            format!("Copied {} characters to the clipboard.", text.chars().count()),
        ),
        Err(_) => {
            let fallback_path = state.cwd.join(".puffer-last-copy.txt");
            std::fs::write(&fallback_path, &text)?;
            emit_system(
                state,
                session_store,
                format!(
                    "Clipboard unavailable. Wrote {} characters to {}.",
                    text.chars().count(),
                    fallback_path.display()
                ),
            )
        }
    }
}

fn describe_context(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
) -> Result<()> {
    let transcript_chars = state
        .transcript
        .iter()
        .map(|message| message.text.chars().count())
        .sum::<usize>();
    emit_system(
        state,
        session_store,
        format!(
            "Context summary:\nmessages={}\ncharacters={}\nworking_dirs={}\nprompts={}\ntools={}\nskills={}\nplugins={}",
            state.transcript.len(),
            transcript_chars,
            state.working_dirs.len(),
            resources.prompts.len(),
            resources.tools.len(),
            resources.skills.len(),
            resources.plugins.len()
        ),
    )
}

fn describe_git_diff(state: &mut AppState, session_store: &SessionStore) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(&state.cwd)
        .args(["diff", "--stat", "--no-ext-diff"])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if text.is_empty() {
                emit_system(state, session_store, "No git diff detected.".to_string())
            } else {
                emit_system(state, session_store, format!("Git diff:\n{text}"))
            }
        }
        Ok(output) => emit_system(
            state,
            session_store,
            format!(
                "git diff failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ),
        Err(error) => emit_system(
            state,
            session_store,
            format!("git diff could not run: {error}"),
        ),
    }
}

fn run_doctor(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    session_store: &SessionStore,
) -> Result<()> {
    let checks = [
        ("git", ["--version"].as_slice()),
        ("sh", ["-lc", "printf ok"].as_slice()),
        ("tmux", ["-V"].as_slice()),
    ];
    let mut text = String::from("Puffer doctor summary:\n");
    for (program, args) in checks {
        let result = Command::new(program).args(args).output();
        match result {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let line = if stdout.is_empty() { "ok".to_string() } else { stdout };
                let _ = writeln!(&mut text, "{program}: {line}");
            }
            Ok(output) => {
                let _ = writeln!(
                    &mut text,
                    "{program}: unavailable ({})",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            Err(error) => {
                let _ = writeln!(&mut text, "{program}: unavailable ({error})");
            }
        }
    }
    let _ = writeln!(&mut text, "providers={}", providers.providers().count());
    let _ = writeln!(&mut text, "tools={}", resources.tools.len());
    let registry = ToolRegistry::from_resources(resources);
    let _ = writeln!(&mut text, "executable_tools={}", registry.tools().count());
    let _ = writeln!(&mut text, "skills={}", resources.skills.len());
    let _ = writeln!(
        &mut text,
        "auth_provider={}",
        state.current_provider.as_deref().unwrap_or("<unset>")
    );
    for tool in registry.tools() {
        let _ = writeln!(
            &mut text,
            "- {} handler={} approval={} sandbox={}",
            tool.spec.id,
            tool.spec.handler,
            tool.spec
                .approval_policy
                .as_deref()
                .unwrap_or("<unspecified>"),
            tool.spec
                .sandbox_policy
                .as_deref()
                .unwrap_or("<unspecified>")
        );
    }
    emit_system(state, session_store, text)
}

fn execute_skill_command(
    state: &mut AppState,
    resources: &LoadedResources,
    session_store: &SessionStore,
    skill_name: &str,
) -> Result<()> {
    if let Some(skill) = skill_by_name(resources, skill_name) {
        emit_system(
            state,
            session_store,
            format!(
                "Skill {}\n{}\n\n{}",
                skill.value.name, skill.value.description, skill.value.content
            ),
        )
    } else {
        emit_system(state, session_store, format!("Unknown skill {skill_name}."))
    }
}

fn emit_system(state: &mut AppState, session_store: &SessionStore, text: String) -> Result<()> {
    state.push_message(MessageRole::System, text.clone());
    session_store.append_event(state.session.id, TranscriptEvent::SystemMessage { text })?;
    Ok(())
}

fn event_to_rendered_message(event: &TranscriptEvent) -> Option<RenderedMessage> {
    match event {
        TranscriptEvent::UserMessage { text } => Some(RenderedMessage {
            role: MessageRole::User,
            text: text.clone(),
        }),
        TranscriptEvent::AssistantMessage { text } => Some(RenderedMessage {
            role: MessageRole::Assistant,
            text: text.clone(),
        }),
        TranscriptEvent::SystemMessage { text } => Some(RenderedMessage {
            role: MessageRole::System,
            text: text.clone(),
        }),
        TranscriptEvent::CommandInvoked { .. } | TranscriptEvent::SessionRenamed { .. } => None,
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
mod tests {
    use super::*;

    #[test]
    fn command_registry_contains_review_usage_and_resume_alias() {
        let commands = supported_commands();
        assert!(find_command(&commands, "review").is_some());
        assert!(find_command(&commands, "usage").is_some());
        assert!(find_command(&commands, "continue").is_some());
    }
}
