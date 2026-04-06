use super::*;
use insta::assert_snapshot;
use puffer_config::PufferConfig;
use puffer_provider_registry::{
    AuthMode, ModelDescriptor, OAuthCredential, ProviderDescriptor, ProviderRegistry,
};
use puffer_resources::{LoadedItem, SourceInfo, SourceKind, ToolSpec};
use puffer_session_store::SessionMetadata;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::path::PathBuf;
use uuid::Uuid;

#[test]
fn session_lines_include_lineage_tags_and_note() {
    let state = sample_state();
    let lines = session_lines(&state).join("\n");
    assert!(lines.contains("Name: Shipyard"));
    assert!(lines.contains("Parent: abcdefab"));
    assert!(lines.contains("Tags: review,tmux"));
    assert!(lines.contains("Note: finish tool parity"));
}

#[test]
fn tool_lines_include_registry_status_and_shell_activity() {
    let state = sample_state();
    let resources = sample_resources();
    let registry = ToolRegistry::from_resources(&resources);
    let lines = tool_lines(&state, &resources, &registry).join("\n");
    assert!(lines.contains("Configured: 4"));
    assert!(lines.contains("Executable: 3"));
    assert!(lines.contains("Kinds: bash=on read=on write=on"));
    assert!(lines.contains("Shell runs: 1"));
    assert!(lines.contains("Last shell: git status"));
}

#[test]
fn header_snapshot_reports_runtime_counts() {
    let state = sample_state();
    let resources = sample_resources();
    let auth_store = sample_auth_store();
    let registry = ToolRegistry::from_resources(&resources);
    let snapshot = header_lines(&state, &resources, &auth_store, &registry)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_snapshot!(
        snapshot,
        @r"
Puffer Code  session=Shipyard  cwd=puffer  msgs=2
provider=anthropic  model=anthropic/claude-sonnet-4-5  auth=api-key  remote=dockyard@staging
slug=shipyard-run  tags=review,tmux  mascot=Clawd
theme=harbor  effort=high  tools=3/4  prompts=2
"
    );
}

#[test]
fn inspector_snapshot_reports_slash_state() {
    let state = sample_state();
    let resources = sample_resources();
    let providers = sample_providers();
    let auth_store = sample_auth_store();
    let snapshot = inspector_lines(
        &state,
        &resources,
        &providers,
        &auth_store,
        "/re",
        &sample_commands(),
    )
    .join("\n");
    assert_snapshot!(
        snapshot,
        @r"
Provider: anthropic
Model: anthropic/claude-sonnet-4-5
Auth: api-key
Auth providers: anthropic,openai
Theme: harbor
Effort: high
Fast mode: on
Vim mode: on
Status line: on
Providers loaded: 2
Prompts: 2
Resources: skills=2 plugins=1 mcp=1 ides=1
Slash filter: /re
Slash matches: 2 best=/review
"
    );
}

#[test]
fn footer_snapshot_reports_slash_hint_and_tool_state() {
    let state = sample_state();
    let resources = sample_resources();
    let auth_store = sample_auth_store();
    let registry = ToolRegistry::from_resources(&resources);
    let snapshot = footer_lines(
        &state,
        &resources,
        &auth_store,
        &registry,
        "/re",
        &sample_commands(),
    )
    .into_iter()
    .map(|line| line.to_string())
    .collect::<Vec<_>>()
    .join("\n");
    assert_snapshot!(
        snapshot,
        @r"
auth=api-key  provider=anthropic  model=anthropic/claude-sonnet-4-5  transcript=2
tools=3/4 ready  shell-runs=1  fast=on  vim=on  sandbox=workspace-write
slash=/re matches=2 best=/review  Enter submits  Esc clears
"
    );
}

#[test]
fn render_draws_session_tool_and_status_surfaces() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = sample_state();
    let resources = sample_resources();
    let providers = sample_providers();
    let auth_store = sample_auth_store();

    terminal
        .draw(|frame| {
            render(
                frame,
                &state,
                &resources,
                &providers,
                &auth_store,
                "/re",
                3,
                0,
                0,
                None,
                &sample_commands(),
            )
        })
        .unwrap();

    let rendered = terminal_view(&terminal);
    assert!(rendered.contains("Session"));
    assert!(rendered.contains("Tools"));
    assert!(rendered.contains("Inspector"));
    assert!(rendered.contains("Status Line"));
    assert!(rendered.contains("slash=/re matches=2 best=/review"));
    assert!(rendered.contains("Commands"));
}

fn sample_state() -> AppState {
    let mut config = PufferConfig::default();
    config.theme = "harbor".to_string();
    config.default_provider = Some("anthropic".to_string());
    config.default_model = Some("anthropic/claude-sonnet-4-5".to_string());
    config.mascot.display_name = "Clawd".to_string();

    let cwd = PathBuf::from("/tmp/puffer");
    let mut state = AppState::new(
        config,
        cwd.clone(),
        SessionMetadata {
            id: Uuid::parse_str("12345678-1234-5678-1234-567812345678").unwrap(),
            display_name: Some("Shipyard".to_string()),
            cwd,
            created_at_ms: 0,
            updated_at_ms: 0,
            parent_session_id: Some(
                Uuid::parse_str("abcdefab-1111-2222-3333-444455556666").unwrap(),
            ),
            slug: Some("shipyard-run".to_string()),
            tags: vec!["review".to_string(), "tmux".to_string()],
            note: Some("finish tool parity".to_string()),
        },
    );
    state.current_provider = Some("anthropic".to_string());
    state.current_model = Some("anthropic/claude-sonnet-4-5".to_string());
    state.prompt_color = "cyan".to_string();
    state.effort_level = "high".to_string();
    state.fast_mode = true;
    state.vim_mode = true;
    state.remote_name = Some("dockyard".to_string());
    state.remote_environment = Some("staging".to_string());
    state.working_dirs = vec![
        PathBuf::from("/tmp/puffer"),
        PathBuf::from("/tmp/puffer/ref"),
    ];
    state.push_message(MessageRole::User, "!git status");
    state.push_message(MessageRole::Assistant, "working tree clean");
    state
}

fn sample_resources() -> LoadedResources {
    LoadedResources {
        tools: vec![
            tool_spec("bash", "bash", Some("on-request"), Some("workspace-write")),
            tool_spec("read_file", "read_file", None, Some("workspace-write")),
            tool_spec(
                "write_file",
                "write_file",
                Some("on-request"),
                Some("workspace-write"),
            ),
            tool_spec("custom_fetch", "custom_fetch", None, None),
        ],
        prompts: vec![
            loaded_prompt("review", "Review current work"),
            loaded_prompt("usage", "Show usage"),
        ],
        skills: vec![
            loaded_skill("rust-review", "Rust review guidance"),
            loaded_skill("tmux-parity", "tmux parity guidance"),
        ],
        plugins: vec![loaded_plugin("git", "Git plugin")],
        mcp_servers: vec![loaded_mcp("docs", "Docs server")],
        ides: vec![loaded_ide("zed", "Zed integration")],
        ..LoadedResources::default()
    }
}

fn sample_providers() -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderDescriptor {
        id: "anthropic".to_string(),
        display_name: "Anthropic".to_string(),
        base_url: "https://api.anthropic.com".to_string(),
        default_api: "anthropic-messages".to_string(),
        auth_modes: vec![AuthMode::ApiKey, AuthMode::OAuth],
        headers: Default::default(),
        models: vec![ModelDescriptor {
            id: "claude-sonnet-4-5".to_string(),
            display_name: "Claude Sonnet 4.5".to_string(),
            provider: "anthropic".to_string(),
            api: "anthropic-messages".to_string(),
            context_window: 200_000,
            max_output_tokens: 8_192,
            supports_reasoning: true,
        }],
    });
    registry.register(ProviderDescriptor {
        id: "openai".to_string(),
        display_name: "OpenAI".to_string(),
        base_url: "https://api.openai.com".to_string(),
        default_api: "responses".to_string(),
        auth_modes: vec![AuthMode::ApiKey, AuthMode::OAuth],
        headers: Default::default(),
        models: vec![ModelDescriptor {
            id: "gpt-5".to_string(),
            display_name: "GPT-5".to_string(),
            provider: "openai".to_string(),
            api: "responses".to_string(),
            context_window: 128_000,
            max_output_tokens: 8_192,
            supports_reasoning: true,
        }],
    });
    registry
}

fn sample_auth_store() -> AuthStore {
    let mut store = AuthStore::default();
    store.set_api_key("anthropic", "sk-ant");
    store.set_oauth(
        "openai",
        OAuthCredential {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at_ms: 42,
            account_id: Some("acct-1".to_string()),
            email: Some("dev@example.com".to_string()),
            scopes: vec!["openid".to_string()],
        },
    );
    store
}

fn sample_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            name: "review",
            aliases: &[],
            description: "Review current changes",
            argument_hint: None,
            kind: CommandKind::Prompt,
        },
        CommandSpec {
            name: "rewind",
            aliases: &["checkpoint"],
            description: "Restore to a prior point",
            argument_hint: None,
            kind: CommandKind::Ui,
        },
        CommandSpec {
            name: "usage",
            aliases: &[],
            description: "Show usage limits",
            argument_hint: None,
            kind: CommandKind::Local,
        },
    ]
}

fn tool_spec(
    id: &str,
    handler: &str,
    approval_policy: Option<&str>,
    sandbox_policy: Option<&str>,
) -> LoadedItem<ToolSpec> {
    LoadedItem {
        value: ToolSpec {
            id: id.to_string(),
            name: id.to_string(),
            description: format!("{id} tool"),
            handler: handler.to_string(),
            approval_policy: approval_policy.map(str::to_string),
            sandbox_policy: sandbox_policy.map(str::to_string),
        },
        source_info: SourceInfo {
            path: PathBuf::from(format!("{id}.yaml")),
            kind: SourceKind::Builtin,
        },
    }
}

fn loaded_prompt(id: &str, description: &str) -> LoadedItem<puffer_resources::PromptTemplate> {
    LoadedItem {
        value: puffer_resources::PromptTemplate {
            id: id.to_string(),
            description: description.to_string(),
            template: "template".to_string(),
        },
        source_info: SourceInfo {
            path: PathBuf::from(format!("{id}.yaml")),
            kind: SourceKind::Builtin,
        },
    }
}

fn loaded_skill(id: &str, description: &str) -> LoadedItem<puffer_resources::SkillSpec> {
    LoadedItem {
        value: puffer_resources::SkillSpec {
            name: id.to_string(),
            description: description.to_string(),
            content: "content".to_string(),
            disable_model_invocation: false,
        },
        source_info: SourceInfo {
            path: PathBuf::from(format!("{id}.yaml")),
            kind: SourceKind::Builtin,
        },
    }
}

fn loaded_plugin(id: &str, display_name: &str) -> LoadedItem<puffer_resources::PluginSpec> {
    LoadedItem {
        value: puffer_resources::PluginSpec {
            id: id.to_string(),
            display_name: display_name.to_string(),
            description: "plugin".to_string(),
            commands: Vec::new(),
            skills: Vec::new(),
            mcp_servers: Vec::new(),
        },
        source_info: SourceInfo {
            path: PathBuf::from(format!("{id}.yaml")),
            kind: SourceKind::Builtin,
        },
    }
}

fn loaded_mcp(id: &str, display_name: &str) -> LoadedItem<puffer_resources::McpServerSpec> {
    LoadedItem {
        value: puffer_resources::McpServerSpec {
            id: id.to_string(),
            display_name: display_name.to_string(),
            transport: "stdio".to_string(),
            endpoint: String::new(),
            target: "cargo run".to_string(),
            description: "mcp".to_string(),
        },
        source_info: SourceInfo {
            path: PathBuf::from(format!("{id}.yaml")),
            kind: SourceKind::Builtin,
        },
    }
}

fn loaded_ide(id: &str, display_name: &str) -> LoadedItem<puffer_resources::IdeSpec> {
    LoadedItem {
        value: puffer_resources::IdeSpec {
            id: id.to_string(),
            display_name: display_name.to_string(),
            description: "ide".to_string(),
        },
        source_info: SourceInfo {
            path: PathBuf::from(format!("{id}.yaml")),
            kind: SourceKind::Builtin,
        },
    }
}

fn terminal_view(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let width = buffer.area.width as usize;
    buffer
        .content
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<Vec<_>>()
                .join("")
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
