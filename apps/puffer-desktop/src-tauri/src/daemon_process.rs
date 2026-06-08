//! Process cleanup helpers for replacing stale local daemon instances.

use anyhow::{Context, Result};
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Terminates a lock-owning local `puffer daemon` and its managed children.
#[cfg(unix)]
pub(super) fn terminate_existing_daemon(user_config_dir: &Path) -> Result<bool> {
    let Some(pid) = daemon_lock_owner_pid(user_config_dir)? else {
        return Ok(false);
    };
    if pid == std::process::id() {
        return Ok(false);
    }
    let Some(command) = process_command(pid) else {
        return Ok(true);
    };
    if !process_looks_like_puffer_daemon(&command) {
        return Ok(false);
    }
    let child_pids = child_process_ids(pid);
    let terminated = terminate_process(pid);
    for child_pid in child_pids {
        if let Some(command) = process_command(child_pid) {
            if process_looks_like_daemon_managed_child(&command) {
                let _ = terminate_process(child_pid);
            }
        }
    }
    Ok(terminated)
}

/// Reports that stale-daemon replacement is unsupported on non-Unix targets.
#[cfg(not(unix))]
pub(super) fn terminate_existing_daemon(_user_config_dir: &Path) -> Result<bool> {
    Ok(false)
}

fn daemon_lock_owner_pid(user_config_dir: &Path) -> Result<Option<u32>> {
    let path = user_config_dir.join("daemon.lock");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("reading daemon lock {}", path.display()));
        }
    };
    let pid = raw.trim();
    if pid.is_empty() {
        return Ok(None);
    }
    let parsed = pid
        .parse()
        .with_context(|| format!("parsing daemon lock pid from {}", path.display()))?;
    Ok(Some(parsed))
}

fn process_looks_like_puffer_daemon(command: &str) -> bool {
    let executable_mentions_puffer = command.starts_with("puffer ") || command.contains("/puffer ");
    executable_mentions_puffer && command.contains(" daemon")
}

fn process_looks_like_daemon_managed_child(command: &str) -> bool {
    command.contains(" __subscriber ")
        || (command.contains("/Google Chrome.app/") && command.contains(".puffer/browser-profiles"))
}

#[cfg(unix)]
fn child_process_ids(pid: u32) -> Vec<u32> {
    let output = Command::new("pgrep")
        .arg("-P")
        .arg(pid.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

#[cfg(unix)]
fn process_command(pid: u32) -> Option<String> {
    let pid = pid.to_string();
    let output = Command::new("ps")
        .arg("-p")
        .arg(&pid)
        .arg("-o")
        .arg("command=")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let command = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!command.is_empty()).then_some(command)
}

#[cfg(unix)]
fn terminate_process(pid: u32) -> bool {
    if send_process_signal(pid, "-TERM") {
        let _ = wait_for_process_exit(pid, Duration::from_secs(2));
        if process_exists(pid) {
            let _ = send_process_signal(pid, "-KILL");
            let _ = wait_for_process_exit(pid, Duration::from_secs(1));
        }
        return true;
    }
    !process_exists(pid)
}

#[cfg(unix)]
fn send_process_signal(pid: u32, signal: &str) -> bool {
    Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    send_process_signal(pid, "-0")
}

#[cfg(unix)]
fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !process_exists(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puffer_daemon_process_detection_accepts_puffer_daemon() {
        assert!(process_looks_like_puffer_daemon(
            "/Applications/Corbina.app/Contents/MacOS/puffer daemon --print-handshake"
        ));
        assert!(process_looks_like_puffer_daemon(
            "puffer daemon --bind 127.0.0.1:0"
        ));
    }

    #[test]
    fn puffer_daemon_process_detection_rejects_other_commands() {
        assert!(!process_looks_like_puffer_daemon(
            "/Applications/Corbina.app/Contents/MacOS/puffer-desktop"
        ));
        assert!(!process_looks_like_puffer_daemon(
            "/usr/bin/python -m puffer daemon"
        ));
    }

    #[test]
    fn daemon_child_detection_accepts_managed_children() {
        assert!(process_looks_like_daemon_managed_child(
            "/Users/shou/puffer/target/debug/puffer __subscriber telegram-user"
        ));
        assert!(process_looks_like_daemon_managed_child(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome --user-data-dir=/Users/shou/.puffer/browser-profiles/puffer-global"
        ));
    }

    #[test]
    fn daemon_child_detection_rejects_unmanaged_children() {
        assert!(!process_looks_like_daemon_managed_child("sleep 100"));
        assert!(!process_looks_like_daemon_managed_child(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome --user-data-dir=/tmp/profile"
        ));
    }
}
