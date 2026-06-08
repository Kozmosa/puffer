use crate::dtos::ChatAttachmentDto;
use anyhow::{bail, Context, Result};
use puffer_config::ConfigPaths;
use puffer_session_store::{
    AttachmentState, SessionStore, StageAttachmentInput, StoredAttachment, StoredAttachmentKind,
};
use serde::Serialize;
use std::env;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MAX_CHAT_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub(crate) enum ChatAttachmentPreviewDto {
    #[serde(rename_all = "camelCase")]
    Available {
        mime_type: String,
        bytes: Vec<u8>,
    },
    Missing,
    Unsupported,
}

#[allow(clippy::too_many_arguments)]
fn stage_chat_attachment_for_store(
    store: &SessionStore,
    session_id: &str,
    name: String,
    mime_type: String,
    extension: String,
    kind: String,
    bytes: Vec<u8>,
) -> Result<ChatAttachmentDto> {
    if bytes.len() > MAX_CHAT_ATTACHMENT_BYTES {
        bail!("attachments must be 20 MiB or smaller");
    }

    let session_uuid = parse_session_id(session_id)?;
    store
        .load_session(session_uuid)
        .with_context(|| format!("unknown session `{session_id}`"))?;
    let attachment = store.stage_attachment(
        session_uuid,
        StageAttachmentInput {
            name,
            mime_type,
            extension,
            kind: stored_attachment_kind(&kind)?,
            bytes,
        },
    )?;
    Ok(ChatAttachmentDto::from_stored(
        store,
        session_uuid,
        &attachment,
    ))
}

fn read_chat_attachment_preview_for_store(
    store: &SessionStore,
    session_id: &str,
    attachment_id: &str,
) -> Result<ChatAttachmentPreviewDto> {
    let session_uuid = parse_session_id(session_id)?;
    validate_attachment_id(attachment_id)?;
    store
        .load_session(session_uuid)
        .with_context(|| format!("unknown session `{session_id}`"))?;
    let Some(attachment) = load_attachment_or_missing(store, session_uuid, attachment_id)? else {
        return Ok(ChatAttachmentPreviewDto::Missing);
    };
    if attachment.kind != StoredAttachmentKind::Image {
        return Ok(ChatAttachmentPreviewDto::Unsupported);
    }
    if store.attachment_state(session_uuid, &attachment) == AttachmentState::Missing {
        return Ok(ChatAttachmentPreviewDto::Missing);
    }
    match store.read_attachment_preview(session_uuid, &attachment) {
        Ok(preview) => Ok(ChatAttachmentPreviewDto::Available {
            mime_type: preview.mime_type,
            bytes: preview.bytes,
        }),
        Err(_) => Ok(ChatAttachmentPreviewDto::Missing),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
/// Stages one chat attachment in the local session store.
pub(crate) fn stage_chat_attachment(
    session_id: String,
    name: String,
    mime_type: String,
    extension: String,
    kind: String,
    bytes: Vec<u8>,
) -> Result<ChatAttachmentDto, String> {
    let store = session_store().map_err(|error| error.to_string())?;
    stage_chat_attachment_for_store(&store, &session_id, name, mime_type, extension, kind, bytes)
        .map_err(|error| error.to_string())
}

#[tauri::command]
/// Reads preview bytes for one staged image attachment.
pub(crate) fn read_chat_attachment_preview(
    session_id: String,
    attachment_id: String,
) -> Result<ChatAttachmentPreviewDto, String> {
    let store = session_store().map_err(|error| error.to_string())?;
    read_chat_attachment_preview_for_store(&store, &session_id, &attachment_id)
        .map_err(|error| error.to_string())
}

fn session_store() -> Result<SessionStore> {
    let root = workspace_root()?;
    SessionStore::from_paths(&ConfigPaths::discover(&root))
}

fn workspace_root() -> Result<PathBuf> {
    if let Ok(value) = env::var("PUFFER_DESKTOP_ROOT") {
        let path = PathBuf::from(value);
        if path.exists() {
            return Ok(path);
        }
    }

    let current_dir = env::current_dir().context("failed to read current directory")?;
    if let Some(path) = find_workspace_root(&current_dir) {
        return Ok(path);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(path) = find_workspace_root(&manifest_dir) {
        return Ok(path);
    }

    Ok(current_dir)
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|ancestor| ancestor.join(".puffer").exists() || ancestor.join(".git").exists())
        .map(PathBuf::from)
}

fn parse_session_id(session_id: &str) -> Result<Uuid> {
    Uuid::parse_str(session_id).context("invalid session id")
}

fn validate_attachment_id(attachment_id: &str) -> Result<()> {
    let parsed = Uuid::parse_str(attachment_id).context("attachment id must be a UUID")?;
    if parsed.to_string() != attachment_id {
        bail!("attachment id must be canonical UUID");
    }
    Ok(())
}

fn stored_attachment_kind(kind: &str) -> Result<StoredAttachmentKind> {
    match kind {
        "image" => Ok(StoredAttachmentKind::Image),
        "file" => Ok(StoredAttachmentKind::File),
        other => bail!("unsupported attachment kind `{other}`"),
    }
}

fn load_attachment_or_missing(
    store: &SessionStore,
    session_id: Uuid,
    attachment_id: &str,
) -> Result<Option<StoredAttachment>> {
    let metadata_path = store
        .root()
        .join(format!("{session_id}.attachments"))
        .join(attachment_id)
        .join("metadata.json");
    if !metadata_path.exists() {
        return Ok(None);
    }
    let mut attachments =
        store.load_staged_attachments(session_id, &[attachment_id.to_string()])?;
    Ok(attachments.pop())
}

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_config::ConfigPaths;
    use tempfile::tempdir;

    fn temp_store() -> (tempfile::TempDir, SessionStore, String) {
        let temp = tempdir().unwrap();
        let paths = ConfigPaths::discover(temp.path());
        let store = SessionStore::from_paths(&paths).unwrap();
        let session = store.create_session(temp.path().to_path_buf()).unwrap();
        (temp, store, session.id.to_string())
    }

    #[test]
    fn stage_chat_attachment_returns_available_timeline_dto() {
        let (_temp, store, session_id) = temp_store();

        let dto = stage_chat_attachment_for_store(
            &store,
            &session_id,
            " pixel.png ".to_string(),
            "image/png".to_string(),
            "PNG".to_string(),
            "image".to_string(),
            vec![1, 2, 3],
        )
        .unwrap();

        assert_eq!(dto.name, "pixel.png");
        assert_eq!(dto.mime_type, "image/png");
        assert_eq!(dto.size, 3);
        assert_eq!(dto.extension, "PNG");
        assert_eq!(dto.kind, "image");
        assert_eq!(dto.state, "available");
        let session_uuid = uuid::Uuid::parse_str(&session_id).unwrap();
        let stored = store
            .load_staged_attachments(session_uuid, std::slice::from_ref(&dto.id))
            .unwrap()
            .remove(0);
        let expected_path = store.attachment_original_path(session_uuid, &stored);
        let value = serde_json::to_value(&dto).unwrap();
        assert_eq!(value["source"]["kind"], "local_file");
        assert_eq!(
            value["source"]["path"].as_str().unwrap(),
            expected_path.display().to_string()
        );
    }

    #[test]
    fn stage_chat_attachment_rejects_files_larger_than_twenty_mib() {
        let (_temp, store, session_id) = temp_store();

        let error = stage_chat_attachment_for_store(
            &store,
            &session_id,
            "large.bin".to_string(),
            "application/octet-stream".to_string(),
            "BIN".to_string(),
            "file".to_string(),
            vec![0; MAX_CHAT_ATTACHMENT_BYTES + 1],
        )
        .unwrap_err();

        assert!(error.to_string().contains("20 MiB"));
    }

    #[test]
    fn read_chat_attachment_preview_returns_available_image_bytes() {
        let (_temp, store, session_id) = temp_store();
        let dto = stage_chat_attachment_for_store(
            &store,
            &session_id,
            "pixel.png".to_string(),
            "image/png".to_string(),
            "PNG".to_string(),
            "image".to_string(),
            vec![1, 2, 3],
        )
        .unwrap();

        let preview = read_chat_attachment_preview_for_store(&store, &session_id, &dto.id).unwrap();

        assert_eq!(
            preview,
            ChatAttachmentPreviewDto::Available {
                mime_type: "image/png".to_string(),
                bytes: vec![1, 2, 3],
            }
        );
    }

    #[test]
    fn read_chat_attachment_preview_reports_unsupported_for_files() {
        let (_temp, store, session_id) = temp_store();
        let dto = stage_chat_attachment_for_store(
            &store,
            &session_id,
            "notes.md".to_string(),
            "text/markdown".to_string(),
            "MD".to_string(),
            "file".to_string(),
            b"# notes".to_vec(),
        )
        .unwrap();

        let preview = read_chat_attachment_preview_for_store(&store, &session_id, &dto.id).unwrap();

        assert_eq!(preview, ChatAttachmentPreviewDto::Unsupported);
    }

    #[test]
    fn read_chat_attachment_preview_reports_missing_when_original_is_gone() {
        let (_temp, store, session_id) = temp_store();
        let dto = stage_chat_attachment_for_store(
            &store,
            &session_id,
            "lost.png".to_string(),
            "image/png".to_string(),
            "PNG".to_string(),
            "image".to_string(),
            vec![1, 2, 3],
        )
        .unwrap();
        let session_uuid = uuid::Uuid::parse_str(&session_id).unwrap();
        let stored = store
            .load_staged_attachments(session_uuid, std::slice::from_ref(&dto.id))
            .unwrap()
            .remove(0);
        std::fs::remove_file(store.attachment_original_path(session_uuid, &stored)).unwrap();

        let preview = read_chat_attachment_preview_for_store(&store, &session_id, &dto.id).unwrap();

        assert_eq!(preview, ChatAttachmentPreviewDto::Missing);
    }
}
