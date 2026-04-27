//! PTY registry — one user-facing shell per `pty_open` RPC.
//!
//! The desktop Terminal tab uses this to drop the user into a live shell
//! rooted at the session's cwd (think "ssh into the session dir"). The
//! agent's own tool-call terminal output stays separate; this is the human
//! escape hatch.
//!
//! Data flow:
//!   pty_open  → spawn $SHELL (or /bin/bash, /bin/sh) in cwd; return pty_id
//!   reader thread → broadcast `pty:<id>:data` events with base64 payloads
//!   pty_write → write base64-decoded bytes to the master
//!   pty_resize → update PtySize
//!   pty_close → kill child + drop handle
//!
//! base64 keeps NDJSON frames ASCII-safe regardless of what the shell
//! emits (control bytes, non-UTF-8, etc.).

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::daemon::ServerEnvelope;

/// Owns a single live PTY. Dropping this kills the child and joins the
/// reader thread implicitly via the Child/master drops.
pub struct PtyHandle {
    /// Master side — we write user keystrokes here. The reader thread owns
    /// its own `try_clone_reader()` so we don't need to share the reader.
    master: Box<dyn MasterPty + Send>,
    /// Persistent writer for the master side. This must live for the full
    /// PTY lifetime; recreating and dropping it per keystroke can close
    /// stdin and make interactive shells treat one character as EOF.
    writer: Box<dyn Write + Send>,
    /// Cloned killer handle — the live Child lives inside the reader
    /// thread (so `wait()` can drive it) and this is our remote SIGKILL
    /// pathway from `close()`.
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// Reader thread. Joined opportunistically when close() runs so we
    /// don't leak threads; ignored if the child is still alive (close
    /// will drive it to exit by killing the child first).
    reader: Option<JoinHandle<()>>,
}

/// Thread-safe map of pty_id → live handle. The map is stored inside
/// `DaemonState` as `Arc<PtyRegistry>` so the RPC handlers (sync) and the
/// reader thread (also sync, via broadcast::Sender) share one registry.
pub struct PtyRegistry {
    inner: Mutex<HashMap<String, PtyHandle>>,
}

impl PtyRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Spawn a shell in `cwd` and register the resulting PTY. Returns the
    /// opaque pty_id the client uses on subsequent write/resize/close RPCs.
    ///
    /// The reader thread owns a clone of the events broadcast::Sender and
    /// streams `pty:<id>:data` events until the child exits, at which
    /// point it emits a final `pty:<id>:exit` event and removes itself
    /// from the registry.
    pub fn open(
        self: &Arc<Self>,
        events: broadcast::Sender<ServerEnvelope>,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> Result<String> {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("openpty")?;

        let shell = resolve_shell();
        let mut cmd = CommandBuilder::new(&shell);
        cmd.cwd(&cwd);
        // Inherit env — users expect $PATH, $HOME, $TERM (we override TERM
        // to xterm-256color since xterm.js is our renderer).
        for (k, v) in std::env::vars_os() {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("spawn shell {shell}"))?;

        // The slave handle is no longer needed once the child has inherited
        // its file descriptors. Dropping it here avoids keeping an extra
        // reference that would delay PTY teardown.
        drop(pair.slave);

        let pty_id = Uuid::new_v4().to_string();

        let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let writer = pair.master.take_writer().context("take pty writer")?;

        // `Child` isn't Clone, but the cloned killer is Send + Sync and
        // can SIGKILL independently. The live Child moves into the reader
        // thread, which is the only place that blocks on `wait()`.
        let killer = child.clone_killer();

        let events_for_reader = events.clone();
        let id_for_reader = pty_id.clone();
        let registry_for_reader = Arc::clone(self);

        let reader_handle = std::thread::spawn(move || {
            let data_channel = format!("pty:{id_for_reader}:data");
            let exit_channel = format!("pty:{id_for_reader}:exit");
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF — child closed its end
                    Ok(n) => {
                        let encoded = BASE64.encode(&buf[..n]);
                        // Best-effort: if no one's listening, drop the
                        // frame. Keeps the reader thread from deadlocking
                        // on a disconnected UI.
                        let _ = events_for_reader.send(ServerEnvelope::Event {
                            event: data_channel.clone(),
                            payload: json!({ "data": encoded }),
                        });
                    }
                    Err(e) => {
                        // EIO is the normal "pty closed" error on Linux
                        // after the child exits; treat all read errors as
                        // terminal.
                        let _ = e;
                        break;
                    }
                }
            }

            let exit_code = match child.wait() {
                Ok(status) => status.exit_code() as i64,
                Err(_) => -1,
            };
            let _ = events_for_reader.send(ServerEnvelope::Event {
                event: exit_channel,
                payload: json!({ "exitCode": exit_code }),
            });

            // Drop our handle from the registry so memory doesn't grow
            // unbounded across many terminal-tab lifecycles.
            let mut guard = registry_for_reader.inner.lock().unwrap();
            guard.remove(&id_for_reader);
        });

        let handle = PtyHandle {
            master: pair.master,
            writer,
            killer,
            reader: Some(reader_handle),
        };
        self.inner.lock().unwrap().insert(pty_id.clone(), handle);
        Ok(pty_id)
    }

    pub fn write(&self, pty_id: &str, data_b64: &str) -> Result<()> {
        let bytes = BASE64
            .decode(data_b64.as_bytes())
            .context("decode pty data")?;
        let mut guard = self.inner.lock().unwrap();
        let handle = guard
            .get_mut(pty_id)
            .with_context(|| format!("no pty `{pty_id}`"))?;
        handle.writer.write_all(&bytes).context("write to pty")?;
        handle.writer.flush().ok();
        Ok(())
    }

    pub fn resize(&self, pty_id: &str, cols: u16, rows: u16) -> Result<()> {
        let guard = self.inner.lock().unwrap();
        let handle = guard
            .get(pty_id)
            .with_context(|| format!("no pty `{pty_id}`"))?;
        handle
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("resize pty")?;
        Ok(())
    }

    pub fn close(&self, pty_id: &str) -> Result<()> {
        let mut handle = {
            let mut guard = self.inner.lock().unwrap();
            match guard.remove(pty_id) {
                Some(h) => h,
                None => return Ok(()), // idempotent
            }
        };
        // Kill the child first so the reader thread wakes from its blocking
        // read and exits; then attempt a non-blocking join.
        let _ = handle.killer.kill();
        // Drop the master explicitly so the reader sees EOF even on
        // platforms where kill() is asynchronous.
        drop(handle.master);
        if let Some(reader) = handle.reader.take() {
            // Don't wait forever — the reader might be blocked on something
            // exotic. If it doesn't join promptly we let the OS reclaim it
            // when the daemon exits.
            let _ = reader.join();
        }
        Ok(())
    }
}

/// Pick the shell to spawn. Prefer the user's $SHELL, then bash, then sh.
/// Resolves the shell to spawn in a user-facing PTY. Honors $SHELL first,
/// then falls back to platform-appropriate defaults so the Terminal tab
/// works out-of-the-box on macOS, Linux, and Windows.
fn resolve_shell() -> String {
    if let Ok(sh) = std::env::var("SHELL") {
        if !sh.trim().is_empty() {
            return sh;
        }
    }
    #[cfg(windows)]
    {
        if let Ok(comspec) = std::env::var("COMSPEC") {
            if !comspec.trim().is_empty() {
                return comspec;
            }
        }
        for candidate in [
            r"C:\Windows\System32\cmd.exe",
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
        ] {
            if std::path::Path::new(candidate).exists() {
                return candidate.to_string();
            }
        }
        return "cmd.exe".to_string();
    }
    #[cfg(not(windows))]
    {
        for candidate in ["/bin/bash", "/bin/sh"] {
            if std::path::Path::new(candidate).exists() {
                return candidate.to_string();
            }
        }
        "/bin/sh".to_string()
    }
}
