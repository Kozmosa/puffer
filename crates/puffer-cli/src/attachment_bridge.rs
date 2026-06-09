//! Bridges uploaded chat attachments to agent-readable local paths.
//!
//! The desktop agent turn receives only a text placeholder like
//! `[Image: 1.jpg]`. Without a real path the model guesses where images
//! live (e.g. the TCC-protected `~/Pictures/Photos Library.photoslibrary`)
//! and crashes. This module materializes each staged attachment to a temp
//! path the agent's `Read` / `VisionAnalyze` tools can access, and builds
//! the model-facing message text that references those paths.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use puffer_session_store::{SessionStore, StoredAttachment, StoredAttachmentKind};
use uuid::Uuid;

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

/// Base temp directory for an attachment, scoped by session and attachment id.
/// `/tmp` is always in the agent's readable workspace roots (see
/// `puffer-core/workspace_paths.rs`), so files here are readable in any sandbox
/// mode without polluting the user's workspace.
fn attachment_temp_dir(session_id: Uuid, attachment_id: &str) -> PathBuf {
    PathBuf::from("/tmp/puffer-attachments")
        .join(session_id.to_string())
        .join(attachment_id)
}

/// Materializes every attachment to a temp path. Best-effort: an attachment
/// whose source is missing or whose copy fails gets `path: None` rather than
/// failing the whole turn.
pub(crate) fn materialize_attachments(
    store: &SessionStore,
    session_id: Uuid,
    attachments: &[StoredAttachment],
) -> Vec<MaterializedAttachment> {
    attachments
        .iter()
        .map(|attachment| MaterializedAttachment {
            attachment: attachment.clone(),
            path: materialize_one(store, session_id, attachment).ok(),
        })
        .collect()
}

fn materialize_one(
    store: &SessionStore,
    session_id: Uuid,
    attachment: &StoredAttachment,
) -> Result<PathBuf> {
    let src = store.attachment_original_path(session_id, attachment);
    if !src.is_file() {
        anyhow::bail!("staged attachment original missing: {}", src.display());
    }
    let dir = attachment_temp_dir(session_id, &attachment.id);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create attachment temp dir {}", dir.display()))?;
    let dest = dir.join(safe_filename(&attachment.name, &attachment.extension));

    // Idempotent: skip the copy if a same-size file is already there.
    if let Ok(meta) = std::fs::metadata(&dest) {
        if meta.len() == attachment.size {
            return Ok(dest);
        }
    }
    std::fs::copy(&src, &dest)
        .with_context(|| format!("copy attachment to {}", dest.display()))?;
    Ok(dest)
}

/// Removes the temp directory tree for a session's materialized attachments.
/// Best-effort; errors are ignored. Call when a session is deleted.
pub(crate) fn cleanup_session_attachments(session_id: Uuid) {
    let dir = PathBuf::from("/tmp/puffer-attachments").join(session_id.to_string());
    let _ = std::fs::remove_dir_all(dir);
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
    use puffer_session_store::StageAttachmentInput;

    fn test_store(temp: &Path) -> (SessionStore, Uuid) {
        let user_config = temp.join(".puffer");
        std::fs::create_dir_all(&user_config).unwrap();
        let paths = puffer_config::ConfigPaths {
            workspace_root: temp.to_path_buf(),
            workspace_config_dir: temp.join("ws.puffer"),
            user_config_dir: user_config,
            builtin_resources_dir: temp.join("resources"),
        };
        (SessionStore::from_paths(&paths).unwrap(), Uuid::new_v4())
    }

    fn stage(store: &SessionStore, session: Uuid, name: &str) -> StoredAttachment {
        store
            .stage_attachment(
                session,
                StageAttachmentInput {
                    name: name.to_string(),
                    mime_type: "image/png".to_string(),
                    extension: "PNG".to_string(),
                    kind: StoredAttachmentKind::Image,
                    bytes: vec![1, 2, 3],
                },
            )
            .unwrap()
    }

    #[test]
    fn materialize_writes_temp_file_with_extension() {
        let temp = tempfile::tempdir().unwrap();
        let (store, session) = test_store(temp.path());
        let att = stage(&store, session, "shot.png");

        let out = materialize_attachments(&store, session, std::slice::from_ref(&att));
        let path = out[0].path.clone().expect("materialized path");

        assert!(path.starts_with("/tmp/puffer-attachments"));
        assert_eq!(path.file_name().unwrap(), "shot.png");
        assert_eq!(std::fs::read(&path).unwrap(), vec![1, 2, 3]);
        cleanup_session_attachments(session);
    }

    #[test]
    fn materialize_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let (store, session) = test_store(temp.path());
        let att = stage(&store, session, "shot.png");

        let first = materialize_attachments(&store, session, std::slice::from_ref(&att))[0]
            .path
            .clone()
            .unwrap();
        let second = materialize_attachments(&store, session, std::slice::from_ref(&att))[0]
            .path
            .clone()
            .unwrap();
        assert_eq!(first, second);
        cleanup_session_attachments(session);
    }

    #[test]
    fn materialize_missing_original_yields_none() {
        let temp = tempfile::tempdir().unwrap();
        let (store, session) = test_store(temp.path());
        let mut att = stage(&store, session, "shot.png");
        att.id = "11111111-1111-1111-1111-111111111111".to_string(); // never staged

        let out = materialize_attachments(&store, session, std::slice::from_ref(&att));
        assert!(out[0].path.is_none());
        cleanup_session_attachments(session);
    }

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
