//! Bridges uploaded chat attachments to agent-readable local paths.
//!
//! The desktop agent turn receives only a text placeholder like
//! `[Image: 1.jpg]`. Without a real path the model guesses where images
//! live (e.g. the TCC-protected `~/Pictures/Photos Library.photoslibrary`)
//! and crashes. This module materializes each staged attachment to a temp
//! path the agent's `Read` / `VisionAnalyze` tools can access, and builds
//! the model-facing message text that references those paths.

use std::path::{Path, PathBuf};

/// Returns a path-safe basename for an attachment, guaranteed to contain no
/// path separators or `..` components, with a file extension when possible.
fn safe_filename(name: &str, extension: &str) -> String {
    // Take the last path component only — defends against names like
    // "../../etc/passwd" or "a/b.png".
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim();
    let base = if base.is_empty() || base == "." || base == ".." {
        ""
    } else {
        base
    };

    let ext = extension.trim().trim_start_matches('.').to_ascii_lowercase();

    if base.is_empty() {
        return if ext.is_empty() {
            "attachment".to_string()
        } else {
            format!("attachment.{ext}")
        };
    }

    if base.contains('.') || ext.is_empty() {
        base.to_string()
    } else {
        format!("{base}.{ext}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_filename_keeps_plain_name() {
        assert_eq!(safe_filename("pixel.png", "PNG"), "pixel.png");
    }

    #[test]
    fn safe_filename_strips_path_components() {
        assert_eq!(safe_filename("../../etc/passwd", ""), "passwd");
        assert_eq!(safe_filename("a/b/c.jpg", "JPG"), "c.jpg");
    }

    #[test]
    fn safe_filename_appends_extension_when_missing() {
        assert_eq!(safe_filename("report", "PDF"), "report.pdf");
    }

    #[test]
    fn safe_filename_falls_back_when_empty_or_dotdot() {
        assert_eq!(safe_filename("..", "PNG"), "attachment.png");
        assert_eq!(safe_filename("", ""), "attachment");
        assert_eq!(safe_filename("   ", "TXT"), "attachment.txt");
    }
}
