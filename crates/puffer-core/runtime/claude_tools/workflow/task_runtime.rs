use super::store::{
    load_store, now_ms, process_is_running, save_store, tasks_path, StoredTask, TaskStore,
};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};

/// Captures child-process output together with timeout state.
pub(super) struct TimedProcessOutput {
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) timed_out: bool,
}

/// Returns true when the stored task status is terminal.
pub(super) fn terminal_task_status(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "stopped" | "deleted")
}

/// Reads persisted task output from disk or the in-memory cache.
pub(super) fn read_task_output(task: &StoredTask) -> Option<String> {
    task.output_file
        .as_deref()
        .and_then(|path| fs::read_to_string(path).ok())
        .filter(|text| !text.is_empty())
        .or_else(|| task.output.clone())
}

/// Refreshes one stored task from disk-backed process and output state.
pub(super) fn refresh_stored_task(store_cwd: &Path, task_id: &str) -> Result<Option<StoredTask>> {
    let mut store = load_store::<TaskStore>(&tasks_path(store_cwd))?;
    let Some(task) = store.tasks.iter_mut().find(|task| task.task_id == task_id) else {
        return Ok(None);
    };

    let mut changed = false;
    if task.status == "running" {
        if let Some(process_id) = task.process_id {
            if !process_is_running(process_id) {
                task.process_id = None;
                task.status = "completed".to_string();
                task.output = read_task_output(task);
                task.updated_at_ms = Some(now_ms());
                changed = true;
            }
        }
    } else if task.output.is_none() {
        task.output = read_task_output(task);
        changed = task.output.is_some();
    }

    let output = task.clone();
    if changed {
        save_store(&tasks_path(store_cwd), &store)?;
    }
    Ok(Some(output))
}

/// Waits until one stored task reaches a terminal state or the timeout elapses.
pub(super) fn wait_for_stored_task(
    store_cwd: &Path,
    task_id: &str,
    timeout_ms: u64,
) -> Result<(Option<StoredTask>, bool)> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let task = refresh_stored_task(store_cwd, task_id)?;
        if task
            .as_ref()
            .is_some_and(|task| terminal_task_status(&task.status))
        {
            return Ok((task, false));
        }
        if Instant::now() >= deadline {
            return Ok((task, true));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Waits for a child process to finish and captures its output with a timeout.
pub(super) fn wait_for_child_output(
    mut child: Child,
    timeout_ms: u64,
) -> Result<TimedProcessOutput> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if child
            .try_wait()
            .context("failed to poll child process")?
            .is_some()
        {
            let output = child
                .wait_with_output()
                .context("failed to collect child output")?;
            return Ok(TimedProcessOutput {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                timed_out: false,
            });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .context("failed to collect timed out child output")?;
            let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if !stderr.trim().is_empty() {
                stderr.push('\n');
            }
            stderr.push_str(&format!("command timed out after {timeout_ms}ms"));
            return Ok(TimedProcessOutput {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr,
                timed_out: true,
            });
        }
        thread::sleep(Duration::from_millis(10));
    }
}
