//! Bridges uploaded chat attachments to agent-readable local paths.
//!
//! The desktop agent turn receives only a text placeholder like
//! `[Image: 1.jpg]`. Without a real path the model guesses where images
//! live (e.g. the TCC-protected `~/Pictures/Photos Library.photoslibrary`)
//! and crashes. This module materializes each staged attachment to a temp
//! path the agent's `Read` / `VisionAnalyze` tools can access, and builds
//! the model-facing message text that references those paths.

use std::path::{Path, PathBuf};

use puffer_session_store::{StoredAttachment, StoredAttachmentKind};

/// A staged attachment plus the temp path it was materialized to (`None` if
/// the source bytes were missing or the copy failed).
pub(crate) struct MaterializedAttachment {
    pub attachment: StoredAttachment,
    pub path: Option<PathBuf>,
}

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

/// Builds the text the model receives: the original message plus a labeled
/// block listing each attachment's local path. Returns the original
/// unchanged when there are no attachments.
pub(crate) fn build_model_input(
    original: &str,
    materialized: &[MaterializedAttachment],
) -> String {
    if materialized.is_empty() {
        return original.to_string();
    }

    let lines: Vec<String> = materialized
        .iter()
        .map(|m| {
            let label = match m.attachment.kind {
                StoredAttachmentKind::Image => "Image",
                StoredAttachmentKind::File => "File",
            };
            match &m.path {
                Some(path) => {
                    let hint = match m.attachment.kind {
                        StoredAttachmentKind::Image => {
                            "read this path with VisionAnalyze or Read"
                        }
                        StoredAttachmentKind::File => "read this path with Read",
                    };
                    format!(
                        "[{label}: {} \u{2192} {} \u{2014} {hint}]",
                        m.attachment.name,
                        path.display()
                    )
                }
                None => format!("[{label}: {} (unavailable)]", m.attachment.name),
            }
        })
        .collect();

    let mut out = original.to_string();
    if !out.trim().is_empty() {
        out.push_str("\n\n");
    }
    out.push_str("Attached files (already saved locally):\n");
    out.push_str(&lines.join("\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(name: &str, kind: StoredAttachmentKind) -> StoredAttachment {
        StoredAttachment {
            id: "00000000-0000-0000-0000-000000000001".to_string(),
            name: name.to_string(),
            mime_type: "application/octet-stream".to_string(),
            size: 3,
            extension: "BIN".to_string(),
            kind,
            storage_key: "k".to_string(),
        }
    }

    #[test]
    fn build_model_input_passthrough_when_no_attachments() {
        assert_eq!(build_model_input("hello", &[]), "hello");
    }

    #[test]
    fn build_model_input_appends_image_path_with_hint() {
        let m = vec![MaterializedAttachment {
            attachment: attachment("1.jpg", StoredAttachmentKind::Image),
            path: Some(PathBuf::from("/tmp/puffer-attachments/s/i/1.jpg")),
        }];
        let out = build_model_input("analyze this image", &m);
        assert!(out.starts_with("analyze this image"));
        assert!(out.contains("/tmp/puffer-attachments/s/i/1.jpg"));
        assert!(out.contains("VisionAnalyze"));
    }

    #[test]
    fn build_model_input_marks_missing_unavailable() {
        let m = vec![MaterializedAttachment {
            attachment: attachment("gone.pdf", StoredAttachmentKind::File),
            path: None,
        }];
        let out = build_model_input("see file", &m);
        assert!(out.contains("[File: gone.pdf (unavailable)]"));
        assert!(!out.contains("\u{2192}"));
    }

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
