//! C string error buffer used by native CEF FFI calls.

use anyhow::{bail, Result};
use std::ffi::{c_char, CStr};

pub(super) struct ErrorBuffer {
    bytes: Vec<c_char>,
}

impl ErrorBuffer {
    /// Creates a fixed-size buffer large enough for native CEF error messages.
    pub(super) fn new() -> Self {
        Self {
            bytes: vec![0; 2048],
        }
    }

    /// Returns a mutable pointer suitable for C APIs that write an error.
    pub(super) fn as_mut_ptr(&mut self) -> *mut c_char {
        self.bytes.as_mut_ptr()
    }

    /// Returns the writable byte length of the C error buffer.
    pub(super) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Converts the C error buffer into a trimmed Rust string.
    pub(super) fn message(&self) -> String {
        unsafe { CStr::from_ptr(self.bytes.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string()
    }

    /// Converts the native success flag into an anyhow result.
    pub(super) fn result(self, ok: i32, context: &str) -> Result<()> {
        if ok != 0 {
            return Ok(());
        }
        let message = self.message();
        if message.is_empty() {
            bail!("{context} failed");
        }
        bail!("{context}: {message}")
    }
}
