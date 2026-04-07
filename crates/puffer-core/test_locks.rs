use std::sync::{Mutex, OnceLock};

/// Returns the shared test mutex used for process-wide environment mutation.
pub(crate) fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
