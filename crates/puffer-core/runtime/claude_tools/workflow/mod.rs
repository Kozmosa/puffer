pub mod agent;
pub mod ask_user_question;
pub mod config;
pub mod cron_create;
pub mod cron_delete;
pub mod cron_list;
pub mod enter_plan_mode;
pub mod enter_worktree;
pub mod exit_plan_mode;
pub mod exit_worktree;
pub mod lsp;
mod lsp_live;
mod lsp_live_diagnostics;
pub mod powershell;
pub mod send_message;
pub mod send_user_message;
pub mod structured_output;
pub mod task_create;
pub mod task_get;
pub mod task_list;
pub mod task_output;
pub mod task_stop;
pub mod task_update;
pub mod team_create;
pub mod team_delete;
pub mod todo_write;

pub(crate) mod store;
mod support;
mod task_runtime;
mod task_tools;

use anyhow::Result;
use std::path::Path;

/// Registers one background Bash task in the shared workflow task store.
pub(crate) fn register_background_shell_task(
    cwd: &Path,
    task_id: &str,
    subject: &str,
    command: &str,
    process_id: u32,
    output_file: &Path,
) -> Result<()> {
    support::register_background_shell_task(cwd, task_id, subject, command, process_id, output_file)
}

/// Marks a background shell task as completed after its process exits.
/// Called from the reaper thread in bash.rs.
pub(crate) fn mark_shell_task_completed(
    cwd: &Path,
    task_id: &str,
    exit_code: Option<i32>,
) -> Result<()> {
    support::mark_shell_task_completed(cwd, task_id, exit_code)
}

/// Returns descriptions of background shell tasks that finished since last check.
/// Each call returns newly completed tasks and clears their "pending notification" flag.
pub(crate) fn drain_completed_shell_tasks(cwd: &Path) -> Vec<String> {
    support::drain_completed_shell_tasks(cwd)
}

/// Returns the number of background shell tasks currently running.
pub(crate) fn running_shell_task_count(cwd: &Path) -> usize {
    // Best-effort: if the workflow directory doesn't exist or isn't writable
    // (e.g. in test environments), return 0.
    let tasks_path = match store::workflow_root_opt(cwd) {
        Some(root) => root.join("tasks.json"),
        None => return 0,
    };
    store::load_store::<store::TaskStore>(&tasks_path)
        .map(|s| {
            s.tasks
                .iter()
                .filter(|t| t.status == "running" || t.status == "in_progress")
                .count()
        })
        .unwrap_or(0)
}
