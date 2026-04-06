mod authflow;

use anyhow::Result;
use clap::{Parser, Subcommand};
use puffer_config::{ensure_workspace_dirs, load_config, ConfigPaths};
use puffer_core::{supported_commands, AppState, MessageRole};
use puffer_provider_openai::{
    build_authorization_url as build_openai_authorization_url,
    exchange_authorization_code as exchange_openai_code, generate_pkce as generate_openai_pkce,
    parse_authorization_input as parse_openai_authorization_input,
    refresh_oauth_token as refresh_openai_oauth_token, OpenAIOAuthConfig,
};
use puffer_provider_registry::{AuthStore, OAuthCredential, ProviderRegistry, StoredCredential};
use puffer_resources::load_resources;
use puffer_session_store::{SessionRecord, SessionStore, TranscriptEvent};
use puffer_tools::ToolRegistry;
use puffer_transport_anthropic::{
    build_authorization_url as build_anthropic_authorization_url, build_messages_request,
    exchange_authorization_code as exchange_anthropic_code,
    generate_pkce as generate_anthropic_pkce,
    parse_authorization_input as parse_anthropic_authorization_input,
    refresh_oauth_token as refresh_anthropic_oauth_token, AnthropicAuth, AnthropicMessage,
    AnthropicModelRequest, AnthropicOAuthConfig, AnthropicRequestConfig,
};
use std::io::Read as _;
use std::io::Write as _;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    bin_name = "puffer",
    subcommand_negates_reqs = true,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    subcommand: Option<Command>,

    /// Optional prompt to submit immediately when launching the TUI.
    prompt: Option<String>,

    /// Disable alternate-screen mode for the TUI.
    #[arg(long = "no-alt-screen", default_value_t = false)]
    no_alt_screen: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Manage stored provider credentials.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// List the supported slash commands.
    Commands,
    /// List known providers and models.
    Providers,
    /// Inspect or modify stored sessions.
    Sessions {
        #[command(subcommand)]
        command: Option<SessionCommand>,
    },
    /// List loaded declarative resources.
    Resources,
    /// Print a sample Anthropic request fixture from the current config.
    AnthropicRequestFixture,
    /// Execute a built-in tool directly for local testing.
    Tool {
        #[command(subcommand)]
        command: ToolCommand,
    },
    /// Print a stored session as JSON.
    Resume {
        /// The session UUID to resume.
        session_id: String,
    },
    /// Fork a stored session into the current working directory.
    Fork {
        /// The session UUID to fork and open.
        session_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum AuthCommand {
    /// Show stored credential status.
    Status,
    /// Store an API key for a provider.
    SetApiKey {
        /// Provider id, for example `anthropic` or `openai`.
        provider: String,
        /// API key value. Omit and use `--stdin` to read from stdin.
        api_key: Option<String>,
        /// Read the API key from stdin.
        #[arg(long = "stdin", default_value_t = false)]
        stdin: bool,
    },
    /// Remove any stored credentials for a provider.
    Clear {
        /// Provider id to clear.
        provider: String,
    },
    /// Print an OAuth authorization URL for a provider when supported.
    OauthUrl {
        /// Provider id to authorize.
        provider: String,
    },
    /// Run a terminal-guided OAuth login flow for a provider.
    Login {
        /// Provider id to authorize.
        provider: String,
        /// Optional pasted callback URL or authorization code.
        value: Option<String>,
        /// Read the callback URL or authorization code from stdin.
        #[arg(long = "stdin", default_value_t = false)]
        stdin: bool,
    },
    /// Generate a full PKCE OAuth start bundle for a provider.
    OauthStart {
        /// Provider id to authorize.
        provider: String,
    },
    /// Exchange a pasted OAuth authorization code for stored credentials.
    OauthExchange {
        /// Provider id to authorize.
        provider: String,
        /// Authorization code or callback URL.
        value: Option<String>,
        /// PKCE code verifier from the `oauth-start` output.
        #[arg(long = "verifier")]
        verifier: String,
        /// Explicit OAuth state for providers that require it.
        #[arg(long = "state")]
        state: Option<String>,
        /// Read the callback URL or code from stdin.
        #[arg(long = "stdin", default_value_t = false)]
        stdin: bool,
    },
    /// Refresh stored OAuth credentials for a provider.
    OauthRefresh {
        /// Provider id to refresh.
        provider: String,
    },
}

#[derive(Debug, Subcommand)]
enum ToolCommand {
    /// List the registered built-in tools and their policies.
    List,
    /// Run a registered tool using a JSON argument payload.
    Run {
        /// Tool id, for example `bash`, `read_file`, or `write_file`.
        tool_id: String,
        /// JSON argument payload.
        args: String,
    },
}

#[derive(Debug, Subcommand)]
enum SessionCommand {
    /// List stored sessions.
    List,
    /// Set or clear the free-form note on a session.
    Note {
        /// Session UUID to update.
        session_id: String,
        /// Note text to store.
        note: Option<String>,
        /// Clear the note instead of setting it.
        #[arg(long = "clear", default_value_t = false)]
        clear: bool,
    },
    /// Add a tag to a session.
    TagAdd {
        /// Session UUID to update.
        session_id: String,
        /// Tag value to add.
        tag: String,
    },
    /// Remove a tag from a session.
    TagRemove {
        /// Session UUID to update.
        session_id: String,
        /// Tag value to remove.
        tag: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    let paths = ConfigPaths::discover(&cwd);
    ensure_workspace_dirs(&paths)?;
    let config = load_config(&paths)?;
    let resources = load_resources(&paths)?;

    let mut providers = ProviderRegistry::new();
    for provider in &resources.providers {
        providers.register_with_source(
            provider.value.clone().into_descriptor(),
            provider.source_info.as_provider_source(),
        );
    }

    let auth_path = paths.user_config_dir.join("auth.json");
    let mut auth_store = AuthStore::load(&auth_path)?;

    match cli.subcommand {
        Some(Command::Auth { command }) => run_auth_command(command, &mut auth_store, &auth_path),
        Some(Command::Commands) => {
            println!("{}", serde_json::to_string_pretty(&supported_commands())?);
            Ok(())
        }
        Some(Command::Providers) => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &providers
                        .provider_entries()
                        .map(|entry| {
                            serde_json::json!({
                                "provider": entry.descriptor,
                                "source": entry.source,
                            })
                        })
                        .collect::<Vec<_>>(),
                )?
            );
            Ok(())
        }
        Some(Command::Sessions { command }) => run_session_command(command, &paths),
        Some(Command::Resources) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "prompts": resources.prompts,
                    "tools": resources.tools,
                    "skills": resources.skills,
                    "plugins": resources.plugins,
                    "mcp_servers": resources.mcp_servers,
                    "ides": resources.ides,
                    "mascots": resources.mascots,
                }))?
            );
            Ok(())
        }
        Some(Command::AnthropicRequestFixture) => {
            let request = build_messages_request(
                &AnthropicRequestConfig {
                    base_url: providers
                        .provider("anthropic")
                        .map(|provider| provider.base_url.clone())
                        .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
                    session_id: "fixture-session".to_string(),
                    custom_headers: indexmap::IndexMap::new(),
                    remote_container_id: None,
                    remote_session_id: None,
                    client_app: None,
                    entrypoint: "cli".to_string(),
                    user_type: "external".to_string(),
                    version: "0.1.0".to_string(),
                    workload: None,
                    additional_protection: false,
                    cch_enabled: true,
                    auth: AnthropicAuth::ApiKey("test-key".to_string()),
                    beta_header: None,
                    client_request_id: Some("fixture-request".to_string()),
                },
                &AnthropicModelRequest {
                    model: config
                        .default_model
                        .clone()
                        .unwrap_or_else(|| "claude-sonnet-4-5".to_string()),
                    max_tokens: 1024,
                    messages: vec![AnthropicMessage {
                        role: "user".to_string(),
                        content: "hello".to_string(),
                    }],
                },
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "method": request.method,
                    "url": request.url,
                    "headers": request.headers,
                    "attribution_prefix_block": request.attribution_prefix_block,
                    "body": serde_json::from_str::<serde_json::Value>(&request.body)?,
                }))?
            );
            Ok(())
        }
        Some(Command::Tool { command }) => run_tool_command(command, &resources, &cwd),
        Some(Command::Resume { session_id }) => {
            let session_store = SessionStore::from_paths(&paths)?;
            let session = session_store.load_session(session_id.parse()?)?;
            let mut state = restore_state_from_session(config.clone(), session);
            puffer_tui::run_app(
                &mut state,
                &resources,
                &providers,
                &auth_store,
                &session_store,
                None,
                cli.no_alt_screen || config.ui.no_alt_screen,
            )
        }
        Some(Command::Fork { session_id }) => {
            let session_store = SessionStore::from_paths(&paths)?;
            let session =
                session_store.fork_session(session_id.parse()?, std::env::current_dir()?)?;
            let record = session_store.load_session(session.id)?;
            let mut state = restore_state_from_session(config.clone(), record);
            puffer_tui::run_app(
                &mut state,
                &resources,
                &providers,
                &auth_store,
                &session_store,
                None,
                cli.no_alt_screen || config.ui.no_alt_screen,
            )
        }
        None => {
            let session_store = SessionStore::from_paths(&paths)?;
            let session = session_store.create_session(cwd.clone())?;
            let mut state = AppState::new(config.clone(), cwd, session);
            puffer_tui::run_app(
                &mut state,
                &resources,
                &providers,
                &auth_store,
                &session_store,
                cli.prompt,
                cli.no_alt_screen || config.ui.no_alt_screen,
            )
        }
    }
}

fn run_tool_command(
    command: ToolCommand,
    resources: &puffer_resources::LoadedResources,
    cwd: &std::path::Path,
) -> Result<()> {
    match command {
        ToolCommand::List => {
            let registry = ToolRegistry::from_resources(resources);
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &registry
                        .tools()
                        .map(|tool| {
                            serde_json::json!({
                                "id": tool.spec.id,
                                "name": tool.spec.name,
                                "description": tool.spec.description,
                                "handler": tool.spec.handler,
                                "approval_policy": tool.spec.approval_policy,
                                "sandbox_policy": tool.spec.sandbox_policy,
                            })
                        })
                        .collect::<Vec<_>>(),
                )?
            );
        }
        ToolCommand::Run { tool_id, args } => {
            let registry = ToolRegistry::from_resources(resources);
            let payload: puffer_tools::ToolInput = serde_json::from_str(&args)?;
            let result = registry.execute(&tool_id, cwd, payload)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}

fn run_session_command(command: Option<SessionCommand>, paths: &ConfigPaths) -> Result<()> {
    let session_store = SessionStore::from_paths(paths)?;
    match command.unwrap_or(SessionCommand::List) {
        SessionCommand::List => {
            println!(
                "{}",
                serde_json::to_string_pretty(&session_store.list_sessions()?)?
            );
        }
        SessionCommand::Note {
            session_id,
            note,
            clear,
        } => {
            let id = session_id.parse()?;
            if clear {
                session_store.set_note(id, None)?;
                println!("cleared note for session {session_id}");
            } else {
                let note = note.ok_or_else(|| anyhow::anyhow!("note text is required unless --clear is used"))?;
                session_store.set_note(id, Some(note))?;
                println!("updated note for session {session_id}");
            }
        }
        SessionCommand::TagAdd { session_id, tag } => {
            session_store.add_tag(session_id.parse()?, tag.clone())?;
            println!("added tag `{tag}` to session {session_id}");
        }
        SessionCommand::TagRemove { session_id, tag } => {
            session_store.remove_tag(session_id.parse()?, &tag)?;
            println!("removed tag `{tag}` from session {session_id}");
        }
    }
    Ok(())
}

fn run_auth_command(
    command: AuthCommand,
    auth_store: &mut AuthStore,
    auth_path: &std::path::Path,
) -> Result<()> {
    match command {
        AuthCommand::Status => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &auth_store
                        .provider_ids()
                        .map(|provider_id| {
                            let kind = match auth_store.get(provider_id) {
                                Some(StoredCredential::ApiKey { .. }) => "api_key",
                                Some(StoredCredential::OAuth(_)) => "oauth",
                                None => "unknown",
                            };
                            serde_json::json!({
                                "provider": provider_id,
                                "kind": kind,
                            })
                        })
                        .collect::<Vec<_>>(),
                )?
            );
        }
        AuthCommand::SetApiKey {
            provider,
            api_key,
            stdin,
        } => {
            let key = if stdin {
                read_secret_from_stdin()?
            } else {
                api_key.ok_or_else(|| {
                    anyhow::anyhow!("api key value is required unless --stdin is used")
                })?
            };
            auth_store.set_api_key(provider.clone(), key);
            auth_store.save(auth_path)?;
            println!("stored api key for {provider}");
        }
        AuthCommand::Clear { provider } => {
            let removed = auth_store.remove(&provider);
            auth_store.save(auth_path)?;
            if removed.is_some() {
                println!("cleared credentials for {provider}");
            } else {
                println!("no credentials were stored for {provider}");
            }
        }
        AuthCommand::OauthUrl { provider } => {
            if provider == "openai" {
                println!(
                    "{}",
                    build_openai_authorization_url(&OpenAIOAuthConfig::default())
                );
            } else if provider == "anthropic" {
                println!(
                    "{}",
                    build_anthropic_authorization_url(&AnthropicOAuthConfig::default())
                );
            } else {
                println!("oauth url generation for {provider} is not implemented yet");
            }
        }
        AuthCommand::Login {
            provider,
            value,
            stdin,
        } => {
            run_login_flow(&provider, value, stdin, auth_store, auth_path)?;
        }
        AuthCommand::OauthStart { provider } => {
            if provider == "openai" {
                let pkce = generate_openai_pkce();
                let config = OpenAIOAuthConfig {
                    state: pkce.state.clone(),
                    code_challenge: pkce.challenge.clone(),
                    ..OpenAIOAuthConfig::default()
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "provider": provider,
                        "authorization_url": build_openai_authorization_url(&config),
                        "code_verifier": pkce.verifier,
                        "state": pkce.state,
                        "redirect_uri": config.redirect_uri,
                    }))?
                );
            } else if provider == "anthropic" {
                let pkce = generate_anthropic_pkce();
                let config = AnthropicOAuthConfig {
                    state: pkce.state.clone(),
                    code_challenge: pkce.challenge.clone(),
                    ..AnthropicOAuthConfig::default()
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "provider": provider,
                        "authorization_url": build_anthropic_authorization_url(&config),
                        "code_verifier": pkce.verifier,
                        "state": pkce.state,
                        "redirect_uri": config.redirect_uri,
                    }))?
                );
            } else {
                anyhow::bail!("oauth start is not implemented for {provider}");
            }
        }
        AuthCommand::OauthExchange {
            provider,
            value,
            verifier,
            state,
            stdin,
        } => {
            let input = if stdin {
                read_secret_from_stdin()?
            } else {
                value.ok_or_else(|| {
                    anyhow::anyhow!(
                        "authorization code or callback URL is required unless --stdin is used"
                    )
                })?
            };
            if provider == "openai" {
                let (code, parsed_state) = parse_openai_authorization_input(&input);
                let code = code.ok_or_else(|| {
                    anyhow::anyhow!("could not extract an OpenAI authorization code")
                })?;
                if let Some(expected_state) = state.as_deref() {
                    if parsed_state.as_deref().unwrap_or_default() != expected_state {
                        anyhow::bail!("oauth state mismatch for openai");
                    }
                }
                let credential = exchange_openai_code(&code, &verifier, None)?;
                auth_store.set_oauth(
                    provider.clone(),
                    to_registry_oauth_credential_openai(credential),
                );
            } else if provider == "anthropic" {
                let (code, parsed_state) = parse_anthropic_authorization_input(&input);
                let code = code.ok_or_else(|| {
                    anyhow::anyhow!("could not extract an Anthropic authorization code")
                })?;
                let expected_state = state.clone().unwrap_or_else(|| verifier.clone());
                if parsed_state.as_deref().unwrap_or_default() != expected_state {
                    anyhow::bail!("oauth state mismatch for anthropic");
                }
                let credential = exchange_anthropic_code(&code, &verifier, &expected_state, None)?;
                auth_store.set_oauth(
                    provider.clone(),
                    to_registry_oauth_credential_anthropic(credential),
                );
            } else {
                anyhow::bail!("oauth exchange is not implemented for {provider}");
            }
            auth_store.save(auth_path)?;
            println!("stored oauth credentials for {provider}");
        }
        AuthCommand::OauthRefresh { provider } => {
            let credential = auth_store
                .get(&provider)
                .ok_or_else(|| anyhow::anyhow!("no credentials stored for {provider}"))?;
            let StoredCredential::OAuth(existing) = credential else {
                anyhow::bail!("stored credential for {provider} is not oauth");
            };
            let refreshed = if provider == "openai" {
                to_registry_oauth_credential_openai(refresh_openai_oauth_token(
                    &existing.refresh_token,
                )?)
            } else if provider == "anthropic" {
                to_registry_oauth_credential_anthropic(refresh_anthropic_oauth_token(
                    &existing.refresh_token,
                )?)
            } else {
                anyhow::bail!("oauth refresh is not implemented for {provider}");
            };
            auth_store.set_oauth(provider.clone(), refreshed);
            auth_store.save(auth_path)?;
            println!("refreshed oauth credentials for {provider}");
        }
    }
    Ok(())
}

fn run_login_flow(
    provider: &str,
    value: Option<String>,
    stdin: bool,
    auth_store: &mut AuthStore,
    auth_path: &std::path::Path,
) -> Result<()> {
    let bundle = match provider {
        "openai" => {
            let pkce = generate_openai_pkce();
            let config = OpenAIOAuthConfig {
                state: pkce.state.clone(),
                code_challenge: pkce.challenge.clone(),
                ..OpenAIOAuthConfig::default()
            };
            OauthStartBundle {
                authorization_url: build_openai_authorization_url(&config),
                verifier: pkce.verifier,
                state: pkce.state,
                redirect_uri: config.redirect_uri,
            }
        }
        "anthropic" => {
            let pkce = generate_anthropic_pkce();
            let config = AnthropicOAuthConfig {
                state: pkce.state.clone(),
                code_challenge: pkce.challenge.clone(),
                ..AnthropicOAuthConfig::default()
            };
            OauthStartBundle {
                authorization_url: build_anthropic_authorization_url(&config),
                verifier: pkce.verifier,
                state: pkce.state,
                redirect_uri: config.redirect_uri,
            }
        }
        other => anyhow::bail!("oauth login is not implemented for {other}"),
    };

    println!(
        "Open this URL in your browser:\n\n{}\n",
        bundle.authorization_url
    );
    let input = if stdin {
        read_secret_from_stdin()?
    } else if let Some(value) = value {
        value
    } else if let Some(callback) =
        authflow::wait_for_callback_url(&bundle.redirect_uri, Duration::from_secs(120))?
    {
        callback
    } else {
        read_line_with_prompt("Paste the callback URL or authorization code: ")?
    };

    if provider == "openai" {
        let (code, parsed_state) = parse_openai_authorization_input(&input);
        let code =
            code.ok_or_else(|| anyhow::anyhow!("could not extract an OpenAI authorization code"))?;
        if let Some(parsed_state) = parsed_state {
            if parsed_state != bundle.state {
                anyhow::bail!("oauth state mismatch for openai");
            }
        }
        let credential = exchange_openai_code(&code, &bundle.verifier, None)?;
        auth_store.set_oauth(
            provider.to_string(),
            to_registry_oauth_credential_openai(credential),
        );
    } else {
        let (code, parsed_state) = parse_anthropic_authorization_input(&input);
        let code = code
            .ok_or_else(|| anyhow::anyhow!("could not extract an Anthropic authorization code"))?;
        let parsed_state = parsed_state.unwrap_or_else(|| bundle.state.clone());
        if parsed_state != bundle.state {
            anyhow::bail!("oauth state mismatch for anthropic");
        }
        let credential = exchange_anthropic_code(&code, &bundle.verifier, &bundle.state, None)?;
        auth_store.set_oauth(
            provider.to_string(),
            to_registry_oauth_credential_anthropic(credential),
        );
    }

    auth_store.save(auth_path)?;
    println!("stored oauth credentials for {provider}");
    Ok(())
}

fn read_secret_from_stdin() -> Result<String> {
    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;
    let trimmed = buffer.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("stdin did not contain a credential value");
    }
    Ok(trimmed)
}

fn read_line_with_prompt(prompt: &str) -> Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let trimmed = line.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("no input received");
    }
    Ok(trimmed)
}

struct OauthStartBundle {
    authorization_url: String,
    verifier: String,
    state: String,
    redirect_uri: String,
}

fn restore_state_from_session(
    config: puffer_config::PufferConfig,
    session: SessionRecord,
) -> AppState {
    let cwd = session.metadata.cwd.clone();
    let mut state = AppState::new(config, cwd, session.metadata);
    for event in session.events {
        match event {
            TranscriptEvent::UserMessage { text } => state.push_message(MessageRole::User, text),
            TranscriptEvent::AssistantMessage { text } => {
                state.push_message(MessageRole::Assistant, text)
            }
            TranscriptEvent::SystemMessage { text } => {
                state.push_message(MessageRole::System, text)
            }
            TranscriptEvent::CommandInvoked { name, args } => state.push_message(
                MessageRole::System,
                format!("Command: /{} {}", name, args).trim().to_string(),
            ),
            TranscriptEvent::SessionRenamed { .. } => {}
        }
    }
    state
}

fn to_registry_oauth_credential_openai(
    credential: puffer_provider_openai::OpenAIOAuthCredentials,
) -> OAuthCredential {
    OAuthCredential {
        access_token: credential.access_token,
        refresh_token: credential.refresh_token,
        expires_at_ms: credential.expires_at_ms,
        account_id: credential.account_id,
        email: None,
        scopes: Vec::new(),
    }
}

fn to_registry_oauth_credential_anthropic(
    credential: puffer_transport_anthropic::AnthropicOAuthCredentials,
) -> OAuthCredential {
    OAuthCredential {
        access_token: credential.access_token,
        refresh_token: credential.refresh_token,
        expires_at_ms: credential.expires_at_ms,
        account_id: credential.account_uuid,
        email: credential.email_address,
        scopes: credential.scopes,
    }
}
