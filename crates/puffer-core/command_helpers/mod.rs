mod auth;
mod common;
mod config;
mod ecosystem;
mod session;

pub(crate) use auth::render_login_guidance;
pub(crate) use common::{
    copy_last_message, describe_context, describe_git_diff, emit_system, execute_skill_command,
    list_skills, rewind_transcript, run_doctor, terminal_setup_advice,
};
pub(crate) use config::{
    handle_config_command, handle_hooks_command, handle_keybindings_command,
    handle_permissions_command, handle_sandbox_command, persist_user_model_selection,
};
pub(crate) use ecosystem::{
    handle_agents_command, handle_ide_command, handle_mcp_command, handle_plugin_command,
    reload_plugins_summary,
};
pub(crate) use session::{append_tool_invocations, handle_memory_command, handle_session_command};
