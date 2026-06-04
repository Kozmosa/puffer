# Chat Attachment Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refreshing desktop chat preserves uploaded attachment cards, and missing files still render metadata without opening an image preview.

**Architecture:** Store chat attachments as session-store sidecars plus structured `StoredAttachment` transcript records. Production file staging goes through Tauri IPC into the local session store; daemon and turn JSON-RPC receive only staged attachment IDs and lightweight timeline DTOs.

**Tech Stack:** Rust session store and daemon APIs, Tauri 2 commands, Svelte 5 desktop UI, Playwright, Cargo tests.

---

## Scope Guardrails

- Do not parse `[Image: name]` or `[File: name]` text to synthesize cards.
- Do not send file bytes through daemon WebSocket JSON-RPC.
- Do not add download, system-open, rich PDF preview, cloud sync, remote upload, cross-session deduplication, or background garbage collection.
- Do not rearrange the existing `sessions/<uuid>.session.json` and `sessions/<uuid>.session.jsonl` layout.
- Every new public Rust item must have a doc comment.

## File Map

- Create `crates/puffer-session-store/src/attachments.rs`
  Owns sidecar paths, staging writes, metadata reads, availability checks, and preview bytes.
- Modify `crates/puffer-session-store/src/events.rs`
  Adds `StoredAttachmentKind`, `StoredAttachment`, and `attachments` on `TranscriptEvent::UserMessage`.
- Modify `crates/puffer-session-store/src/lib.rs`
  Re-exports attachment types needed by daemon and desktop crates.
- Modify `crates/puffer-cli/src/desktop_api_types.rs`
  Adds timeline attachment DTOs and attaches them to `TimelineItemDto::UserMessage`.
- Modify `crates/puffer-cli/src/desktop_api.rs`
  Maps stored transcript attachments to timeline DTOs with computed availability.
- Modify `crates/puffer-cli/src/daemon.rs`
  Parses `attachmentIds`, loads staged attachment records, and persists them with the user prompt.
- Create `apps/puffer-desktop/src-tauri/src/chat_attachments.rs`
  Tauri command helpers for staging and preview reads.
- Modify `apps/puffer-desktop/src-tauri/src/lib.rs`
  Registers `stage_chat_attachment` and `read_chat_attachment_preview`.
- Modify `apps/puffer-desktop/src-tauri/src/dtos.rs`, `apps/puffer-desktop/src-tauri/src/dto.rs`
  Adds attachment DTOs for both desktop backend paths.
- Modify `apps/puffer-desktop/src-tauri/src/backend.rs`, `apps/puffer-desktop/src-tauri/src/session_data.rs`, `apps/puffer-desktop/src-tauri/src/session_api.rs`
  Carries attachments through in-process and session-store-backed timeline conversion.
- Modify `apps/puffer-desktop/src/lib/types.ts`
  Adds attachment availability state and staged attachment typing.
- Modify `apps/puffer-desktop/src/lib/api/desktop.ts`
  Adds `stageChatAttachment`, `readChatAttachmentPreview`, and changes `runAgentTurn` to use `attachmentIds`.
- Modify `apps/puffer-desktop/src/lib/screens/agent/attachments.ts`
  Keeps composer drafts local and converts staged refs into message attachments.
- Modify `apps/puffer-desktop/src/lib/agentTurnAttachments.ts`
  Formats prompt fallback lines from staged refs.
- Modify `apps/puffer-desktop/src/App.svelte`
  Stages attachments before submit, persists optimistic cards from staged refs, and sends only IDs to `runAgentTurn`.
- Modify `apps/puffer-desktop/src/lib/screens/agent/AttachmentPreviewStrip.svelte`
  Ensures missing attachments open the existing unavailable detail path rather than image preview.
- Modify `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`
  Reads preview bytes on demand for refreshed image attachments.
- Modify `apps/puffer-desktop/tests/support/fakeDaemon.ts`
  Supports timeline attachment fixtures and preview responses for Playwright tests.
- Modify `apps/puffer-desktop/tests/chat-session-ui.spec.ts`
  Adds hard-refresh card, hidden fallback text, available preview, and missing attachment tests.

## Task 1: Session-Store Attachment Records

**Files:**
- Create: `crates/puffer-session-store/src/attachments.rs`
- Modify: `crates/puffer-session-store/src/events.rs`
- Modify: `crates/puffer-session-store/src/lib.rs`

- [ ] **Step 1: Write failing event serialization tests**

Add this test in `crates/puffer-session-store/src/events.rs` inside the existing test module:

```rust
#[test]
fn user_message_serializes_stored_attachments() {
    let event = TranscriptEvent::UserMessage {
        text: "inspect this".to_string(),
        attachments: vec![StoredAttachment {
            id: "att-1".to_string(),
            name: "diagram.png".to_string(),
            mime_type: "image/png".to_string(),
            size: 68,
            extension: "PNG".to_string(),
            kind: StoredAttachmentKind::Image,
            storage_key: "att-1/original".to_string(),
        }],
        actor: None,
    };

    let value = serde_json::to_value(event).unwrap();
    assert_eq!(value["type"], "user_message");
    assert_eq!(value["attachments"][0]["id"], "att-1");
    assert_eq!(value["attachments"][0]["kind"], "image");
    assert_eq!(value["attachments"][0]["storage_key"], "att-1/original");
}
```

- [ ] **Step 2: Run the failing event test**

Run:

```bash
cargo test -p puffer-session-store user_message_serializes_stored_attachments -- --nocapture
```

Expected: FAIL because `StoredAttachment`, `StoredAttachmentKind`, and the `attachments` field do not exist.

- [ ] **Step 3: Add stored attachment event types**

In `crates/puffer-session-store/src/events.rs`, add these public types above `TranscriptEvent`:

```rust
/// Stored kind for a chat attachment referenced by a transcript user message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StoredAttachmentKind {
    /// Image attachment that can be previewed when the backing file exists.
    Image,
    /// Non-image file attachment shown as metadata in chat.
    File,
}

/// Durable metadata for a staged chat attachment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredAttachment {
    /// Stable attachment id scoped to one session.
    pub id: String,
    /// Display name shown in chat and prompt fallback text.
    pub name: String,
    /// MIME type recorded when the file was staged.
    pub mime_type: String,
    /// Byte length recorded when the file was staged.
    pub size: u64,
    /// Uppercase file extension used by attachment cards.
    pub extension: String,
    /// Attachment rendering and preview category.
    pub kind: StoredAttachmentKind,
    /// Session-store-relative key used to locate the stored file.
    pub storage_key: String,
}
```

Change `TranscriptEvent::UserMessage` to:

```rust
UserMessage {
    text: String,
    attachments: Vec<StoredAttachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor: Option<MessageActor>,
},
```

Update every Rust construction and match of `TranscriptEvent::UserMessage` in the workspace to include or ignore `attachments`. For constructors that do not attach files, use `attachments: Vec::new()`.

- [ ] **Step 4: Export attachment types**

In `crates/puffer-session-store/src/lib.rs`, add:

```rust
mod attachments;

pub use attachments::{
    AttachmentPreviewBytes, AttachmentState, StageAttachmentInput,
};
pub use events::{StoredAttachment, StoredAttachmentKind};
```

The new module will be implemented in the next step; this export should compile after Step 6.

- [ ] **Step 5: Write failing storage tests**

Create `crates/puffer-session-store/src/attachments.rs` with the module skeleton and tests:

```rust
use crate::{SessionStore, StoredAttachment};
use anyhow::Result;
use uuid::Uuid;

/// Availability of a staged attachment's backing file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentState {
    /// The stored file exists.
    Available,
    /// The stored file is missing.
    Missing,
}

/// Bytes returned for an available attachment preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentPreviewBytes {
    /// MIME type associated with the stored file.
    pub mime_type: String,
    /// Raw bytes read from storage.
    pub bytes: Vec<u8>,
}

/// Input used when staging one chat attachment into the session store.
#[derive(Debug, Clone)]
pub struct StageAttachmentInput {
    /// Original display filename.
    pub name: String,
    /// Browser-provided MIME type, normalized by the caller.
    pub mime_type: String,
    /// Uppercase display extension.
    pub extension: String,
    /// Attachment kind.
    pub kind: crate::StoredAttachmentKind,
    /// File bytes to store.
    pub bytes: Vec<u8>,
}

impl SessionStore {
    /// Stages one chat attachment and returns durable metadata.
    pub fn stage_attachment(
        &self,
        _session_id: Uuid,
        _input: StageAttachmentInput,
    ) -> Result<StoredAttachment> {
        anyhow::bail!("stage_attachment not implemented")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StoredAttachmentKind;
    use puffer_config::ConfigPaths;
    use tempfile::tempdir;

    #[test]
    fn stage_attachment_writes_sidecar_and_reports_available() {
        let temp = tempdir().unwrap();
        let paths = ConfigPaths::discover(temp.path());
        let store = SessionStore::from_paths(&paths).unwrap();
        let session = store.create_session(temp.path().to_path_buf()).unwrap();

        let attachment = store
            .stage_attachment(
                session.id,
                StageAttachmentInput {
                    name: "pixel.png".to_string(),
                    mime_type: "image/png".to_string(),
                    extension: "PNG".to_string(),
                    kind: StoredAttachmentKind::Image,
                    bytes: vec![1, 2, 3],
                },
            )
            .unwrap();

        assert_eq!(attachment.name, "pixel.png");
        assert_eq!(store.attachment_state(session.id, &attachment), AttachmentState::Available);
        assert_eq!(
            store.read_attachment_preview(session.id, &attachment).unwrap().bytes,
            vec![1, 2, 3]
        );
    }

    #[test]
    fn missing_original_reports_missing() {
        let temp = tempdir().unwrap();
        let paths = ConfigPaths::discover(temp.path());
        let store = SessionStore::from_paths(&paths).unwrap();
        let session = store.create_session(temp.path().to_path_buf()).unwrap();
        let attachment = store
            .stage_attachment(
                session.id,
                StageAttachmentInput {
                    name: "lost.png".to_string(),
                    mime_type: "image/png".to_string(),
                    extension: "PNG".to_string(),
                    kind: StoredAttachmentKind::Image,
                    bytes: vec![4, 5, 6],
                },
            )
            .unwrap();

        std::fs::remove_file(store.attachment_original_path(session.id, &attachment)).unwrap();

        assert_eq!(store.attachment_state(session.id, &attachment), AttachmentState::Missing);
        assert!(store.read_attachment_preview(session.id, &attachment).is_err());
    }
}
```

- [ ] **Step 6: Run the failing storage tests**

Run:

```bash
cargo test -p puffer-session-store attachment -- --nocapture
```

Expected: FAIL because `attachment_state`, `read_attachment_preview`, and `attachment_original_path` are not implemented.

- [ ] **Step 7: Implement sidecar storage**

Replace the `impl SessionStore` block in `attachments.rs` with working methods:

```rust
impl SessionStore {
    /// Stages one chat attachment and returns durable metadata.
    pub fn stage_attachment(
        &self,
        session_id: Uuid,
        input: StageAttachmentInput,
    ) -> Result<StoredAttachment> {
        let id = Uuid::new_v4().to_string();
        let dir = self.attachment_dir(session_id, &id);
        std::fs::create_dir_all(&dir)?;
        let temp_path = dir.join("original.tmp");
        let original_path = dir.join("original");
        std::fs::write(&temp_path, &input.bytes)?;
        std::fs::rename(&temp_path, &original_path)?;
        let attachment = StoredAttachment {
            id: id.clone(),
            name: sanitize_attachment_name(&input.name),
            mime_type: input.mime_type,
            size: input.bytes.len() as u64,
            extension: input.extension,
            kind: input.kind,
            storage_key: format!("{id}/original"),
        };
        std::fs::write(dir.join("metadata.json"), serde_json::to_vec(&attachment)?)?;
        Ok(attachment)
    }

    /// Loads staged attachment metadata by id.
    pub fn load_staged_attachments(
        &self,
        session_id: Uuid,
        ids: &[String],
    ) -> Result<Vec<StoredAttachment>> {
        ids.iter()
            .map(|id| {
                let path = self.attachment_dir(session_id, id).join("metadata.json");
                let bytes = std::fs::read(&path)?;
                Ok(serde_json::from_slice(&bytes)?)
            })
            .collect()
    }

    /// Returns whether the stored original exists.
    pub fn attachment_state(
        &self,
        session_id: Uuid,
        attachment: &StoredAttachment,
    ) -> AttachmentState {
        if self.attachment_original_path(session_id, attachment).exists() {
            AttachmentState::Available
        } else {
            AttachmentState::Missing
        }
    }

    /// Reads preview bytes for an available attachment.
    pub fn read_attachment_preview(
        &self,
        session_id: Uuid,
        attachment: &StoredAttachment,
    ) -> Result<AttachmentPreviewBytes> {
        let path = self.attachment_original_path(session_id, attachment);
        let bytes = std::fs::read(&path)?;
        Ok(AttachmentPreviewBytes {
            mime_type: attachment.mime_type.clone(),
            bytes,
        })
    }

    /// Returns the original file path for a stored attachment.
    pub fn attachment_original_path(
        &self,
        session_id: Uuid,
        attachment: &StoredAttachment,
    ) -> std::path::PathBuf {
        self.attachment_dir(session_id, &attachment.id).join("original")
    }

    fn attachment_dir(&self, session_id: Uuid, attachment_id: &str) -> std::path::PathBuf {
        self.root()
            .join(format!("{session_id}.attachments"))
            .join(attachment_id)
    }
}

fn sanitize_attachment_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if sanitized.is_empty() {
        "attachment".to_string()
    } else {
        sanitized
    }
}
```

- [ ] **Step 8: Run session-store tests**

Run:

```bash
cargo test -p puffer-session-store
```

Expected: PASS after updating existing `UserMessage` constructors and matches.

- [ ] **Step 9: Commit session-store changes**

Run:

```bash
git add crates/puffer-session-store
git commit -m "feat(session-store): persist chat attachments"
```

## Task 2: Daemon Timeline And Turn Persistence

**Files:**
- Modify: `crates/puffer-cli/src/desktop_api_types.rs`
- Modify: `crates/puffer-cli/src/desktop_api.rs`
- Modify: `crates/puffer-cli/src/daemon.rs`
- Test: `crates/puffer-cli/src/desktop_api.rs`
- Test: `crates/puffer-cli/tests/daemon_turn_smoke.rs`

- [ ] **Step 1: Write failing desktop timeline test**

In `crates/puffer-cli/src/desktop_api.rs`, add this test in the existing test module:

```rust
#[test]
fn timeline_user_message_includes_attachment_state() {
    let temp = tempfile::tempdir().unwrap();
    let paths = puffer_config::ConfigPaths::discover(temp.path());
    let store = SessionStore::from_paths(&paths).unwrap();
    let session = store.create_session(temp.path().to_path_buf()).unwrap();
    let attachment = store
        .stage_attachment(
            session.id,
            puffer_session_store::StageAttachmentInput {
                name: "pixel.png".to_string(),
                mime_type: "image/png".to_string(),
                extension: "PNG".to_string(),
                kind: puffer_session_store::StoredAttachmentKind::Image,
                bytes: vec![1, 2, 3],
            },
        )
        .unwrap();
    store
        .append_event(
            session.id,
            TranscriptEvent::UserMessage {
                text: "[Image: pixel.png]".to_string(),
                attachments: vec![attachment],
                actor: None,
            },
        )
        .unwrap();

    let detail = load_session_detail(&store, &session.id.to_string()).unwrap();
    let user = detail
        .timeline
        .iter()
        .find(|item| matches!(item, TimelineItemDto::UserMessage { .. }))
        .unwrap();

    match user {
        TimelineItemDto::UserMessage { attachments, .. } => {
            assert_eq!(attachments.len(), 1);
            assert_eq!(attachments[0].name, "pixel.png");
            assert_eq!(attachments[0].state, "available");
        }
        _ => unreachable!(),
    }
}
```

- [ ] **Step 2: Run the failing desktop timeline test**

Run:

```bash
cargo test -p puffer-cli timeline_user_message_includes_attachment_state -- --nocapture
```

Expected: FAIL because `TimelineItemDto::UserMessage` has no `attachments` field.

- [ ] **Step 3: Add attachment DTOs**

In `crates/puffer-cli/src/desktop_api_types.rs`, add:

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatAttachmentDto {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) mime_type: String,
    pub(crate) size: u64,
    pub(crate) extension: String,
    pub(crate) kind: String,
    pub(crate) state: String,
}
```

Change `TimelineItemDto::UserMessage` to include:

```rust
attachments: Vec<ChatAttachmentDto>,
```

- [ ] **Step 4: Map stored attachments to DTOs**

In `crates/puffer-cli/src/desktop_api.rs`, change the `timeline_items` signature from:

```rust
fn timeline_items(record: &SessionRecord) -> Vec<TimelineItemDto>
```

to:

```rust
fn timeline_items(session_store: &SessionStore, record: &SessionRecord) -> Vec<TimelineItemDto>
```

Replace the current user-message match arm with:

```rust
TranscriptEvent::UserMessage { text, attachments, actor } => {
    flush_pending_assistant(&mut items, &mut pending_assistant);
    items.push(TimelineItemDto::UserMessage {
        id: format!("timeline-{index}"),
        text: text.clone(),
        attachments: attachment_dtos(session_store, record.metadata.id, attachments),
        actor: actor.clone(),
    });
}
```

Add this helper:

```rust
fn attachment_dtos(
    session_store: &SessionStore,
    session_id: uuid::Uuid,
    attachments: &[puffer_session_store::StoredAttachment],
) -> Vec<ChatAttachmentDto> {
    attachments
        .iter()
        .map(|attachment| {
            let state = match session_store.attachment_state(session_id, attachment) {
                puffer_session_store::AttachmentState::Available => "available",
                puffer_session_store::AttachmentState::Missing => "missing",
            };
            ChatAttachmentDto {
                id: attachment.id.clone(),
                name: attachment.name.clone(),
                mime_type: attachment.mime_type.clone(),
                size: attachment.size,
                extension: attachment.extension.clone(),
                kind: match attachment.kind {
                    puffer_session_store::StoredAttachmentKind::Image => "image".to_string(),
                    puffer_session_store::StoredAttachmentKind::File => "file".to_string(),
                },
                state: state.to_string(),
            }
        })
        .collect()
}
```

- [ ] **Step 5: Write failing daemon turn test**

Add a focused test in `crates/puffer-cli/tests/daemon_turn_smoke.rs` that stages one attachment through `SessionStore`, calls `run_agent_turn` with `attachmentIds`, then loads session detail and asserts the user message includes one attachment. Copy the daemon setup from the nearest existing successful `run_agent_turn` smoke test, then insert this staging block after the session is created:

```rust
let session_uuid = uuid::Uuid::parse_str(session_id).unwrap();
let session_store = puffer_session_store::SessionStore::from_paths(&paths).unwrap();
let attachment = session_store
    .stage_attachment(
        session_uuid,
        puffer_session_store::StageAttachmentInput {
            name: "pixel.png".to_string(),
            mime_type: "image/png".to_string(),
            extension: "PNG".to_string(),
            kind: puffer_session_store::StoredAttachmentKind::Image,
            bytes: vec![1, 2, 3],
        },
    )
    .unwrap();

let turn = client.rpc(
    "run_agent_turn",
    json!({
        "sessionId": session_id,
        "message": "[Image: pixel.png]",
        "attachmentIds": [attachment.id],
        "providerId": "codex",
    }),
);
assert!(turn["turnId"].as_str().unwrap().len() > 0);
```

Use this assertion shape:

```rust
let detail = client.rpc("load_session_detail", json!({ "sessionId": session_id }));
let timeline = detail["timeline"].as_array().unwrap();
let user = timeline
    .iter()
    .find(|item| item["kind"] == "user_message")
    .unwrap();
assert_eq!(user["attachments"][0]["name"], "pixel.png");
assert_eq!(user["attachments"][0]["state"], "available");
```

- [ ] **Step 6: Run the failing daemon turn test**

Run:

```bash
cargo test -p puffer-cli --test daemon_turn_smoke attachment -- --nocapture
```

Expected: FAIL because daemon `run_agent_turn` ignores `attachmentIds`.

- [ ] **Step 7: Persist staged attachments during turn start**

In `crates/puffer-cli/src/daemon.rs`, add a parser:

```rust
fn parse_attachment_ids(params: &Value) -> Result<Vec<String>> {
    let Some(value) = params.get("attachmentIds").or_else(|| params.get("attachment_ids")) else {
        return Ok(Vec::new());
    };
    let ids = value
        .as_array()
        .context("attachmentIds must be an array")?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .context("attachmentIds must contain strings")
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ids)
}
```

In `start_turn`, parse IDs beside `message` and move them into the worker thread. Before appending `TranscriptEvent::UserMessage`, call:

```rust
let staged_attachments = match inputs
    .session_store
    .load_staged_attachments(session_uuid, &attachment_ids_for_thread)
{
    Ok(attachments) => attachments,
    Err(error) => {
        publish_turn_error_event(
            &setup_state,
            &channel_thread,
            &session_id_for_thread,
            &turn_id_thread,
            format!("attachment staging: {error:#}"),
            None,
            None,
        );
        setup_state.turns.lock().unwrap().remove(&turn_id_thread);
        return;
    }
};
```

Then append:

```rust
TranscriptEvent::UserMessage {
    text: message_for_thread.clone(),
    attachments: staged_attachments,
    actor: Some(app_state.user_actor()),
}
```

If loading staged attachments fails, publish the turn error and return before provider execution. Do not persist a user message with missing staged IDs.

- [ ] **Step 8: Run daemon and CLI tests**

Run:

```bash
cargo test -p puffer-cli timeline_user_message_includes_attachment_state
cargo test -p puffer-cli --test daemon_turn_smoke attachment
```

Expected: PASS.

- [ ] **Step 9: Commit daemon changes**

Run:

```bash
git add crates/puffer-cli crates/puffer-session-store
git commit -m "feat(cli): persist staged chat attachments"
```

## Task 3: Tauri Staging And Preview Commands

**Files:**
- Create: `apps/puffer-desktop/src-tauri/src/chat_attachments.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/lib.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/dtos.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/dto.rs`

- [ ] **Step 1: Write failing command registration test**

In `apps/puffer-desktop/src-tauri/src/lib.rs`, extend the existing registered-command test expectations with:

```rust
"stage_chat_attachment",
"read_chat_attachment_preview",
```

Run:

```bash
cargo test -p corbina registered_tauri_commands -- --nocapture
```

Expected: FAIL because the commands are not registered.

- [ ] **Step 2: Add DTOs for attachment responses**

Add this struct to both `apps/puffer-desktop/src-tauri/src/dtos.rs` and `apps/puffer-desktop/src-tauri/src/dto.rs`:

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatAttachmentDto {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) mime_type: String,
    pub(crate) size: u64,
    pub(crate) extension: String,
    pub(crate) kind: String,
    pub(crate) state: String,
}
```

- [ ] **Step 3: Create Tauri attachment command module**

Create `apps/puffer-desktop/src-tauri/src/chat_attachments.rs`:

```rust
use anyhow::{bail, Result};
use puffer_config::ConfigPaths;
use puffer_session_store::{
    AttachmentState, SessionStore, StageAttachmentInput, StoredAttachment,
    StoredAttachmentKind,
};
use serde::Serialize;
use std::path::PathBuf;
use uuid::Uuid;

const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatAttachmentResponse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) mime_type: String,
    pub(crate) size: u64,
    pub(crate) extension: String,
    pub(crate) kind: String,
    pub(crate) state: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub(crate) enum AttachmentPreviewResponse {
    Available {
        #[serde(rename = "mimeType")]
        mime_type: String,
        bytes: Vec<u8>,
    },
    Missing,
    Unsupported,
}

/// Stages one chat attachment into the local session store.
pub(crate) fn stage_chat_attachment(
    session_id: String,
    name: String,
    mime_type: String,
    extension: String,
    kind: String,
    bytes: Vec<u8>,
) -> Result<ChatAttachmentResponse> {
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        bail!("Attachment exceeds 20 MiB.");
    }
    let session_uuid = Uuid::parse_str(&session_id)?;
    let store = local_session_store()?;
    let stored = store.stage_attachment(
        session_uuid,
        StageAttachmentInput {
            name,
            mime_type,
            extension,
            kind: parse_kind(&kind)?,
            bytes,
        },
    )?;
    Ok(to_response(&store, session_uuid, &stored))
}

/// Reads preview bytes for one staged or persisted chat attachment.
pub(crate) fn read_chat_attachment_preview(
    session_id: String,
    attachment_id: String,
) -> Result<AttachmentPreviewResponse> {
    let session_uuid = Uuid::parse_str(&session_id)?;
    let store = local_session_store()?;
    let attachments = match store.load_staged_attachments(session_uuid, &[attachment_id]) {
        Ok(attachments) => attachments,
        Err(_) => return Ok(AttachmentPreviewResponse::Missing),
    };
    let Some(attachment) = attachments.first() else {
        return Ok(AttachmentPreviewResponse::Missing);
    };
    if !matches!(attachment.kind, StoredAttachmentKind::Image) {
        return Ok(AttachmentPreviewResponse::Unsupported);
    }
    if store.attachment_state(session_uuid, attachment) == AttachmentState::Missing {
        return Ok(AttachmentPreviewResponse::Missing);
    }
    let preview = store.read_attachment_preview(session_uuid, attachment)?;
    Ok(AttachmentPreviewResponse::Available {
        mime_type: preview.mime_type,
        bytes: preview.bytes,
    })
}

fn parse_kind(kind: &str) -> Result<StoredAttachmentKind> {
    match kind {
        "image" => Ok(StoredAttachmentKind::Image),
        "file" => Ok(StoredAttachmentKind::File),
        _ => bail!("Unsupported attachment kind: {kind}"),
    }
}

fn to_response(
    store: &SessionStore,
    session_id: Uuid,
    attachment: &StoredAttachment,
) -> ChatAttachmentResponse {
    let state = match store.attachment_state(session_id, attachment) {
        AttachmentState::Available => "available",
        AttachmentState::Missing => "missing",
    };
    ChatAttachmentResponse {
        id: attachment.id.clone(),
        name: attachment.name.clone(),
        mime_type: attachment.mime_type.clone(),
        size: attachment.size,
        extension: attachment.extension.clone(),
        kind: match attachment.kind {
            StoredAttachmentKind::Image => "image".to_string(),
            StoredAttachmentKind::File => "file".to_string(),
        },
        state: state.to_string(),
    }
}

fn local_session_store() -> Result<SessionStore> {
    let root = std::env::var_os("PUFFER_DESKTOP_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    SessionStore::from_paths(&ConfigPaths::discover(&root))
}
```

- [ ] **Step 4: Register Tauri commands**

In `apps/puffer-desktop/src-tauri/src/lib.rs`, add:

```rust
mod chat_attachments;
```

Add command wrappers:

```rust
#[tauri::command]
fn stage_chat_attachment(
    session_id: String,
    name: String,
    mime_type: String,
    extension: String,
    kind: String,
    bytes: Vec<u8>,
) -> Result<Value, String> {
    let response = chat_attachments::stage_chat_attachment(
        session_id, name, mime_type, extension, kind, bytes,
    )
    .map_err(|error| error.to_string())?;
    json_value(response)
}

#[tauri::command]
fn read_chat_attachment_preview(
    session_id: String,
    attachment_id: String,
) -> Result<Value, String> {
    let response = chat_attachments::read_chat_attachment_preview(
        session_id,
        attachment_id,
    )
    .map_err(|error| error.to_string())?;
    json_value(response)
}
```

Register both in the `invoke_handler!` list and `REGISTERED_TAURI_COMMANDS`.

- [ ] **Step 5: Run Tauri crate tests**

Run:

```bash
cargo test -p corbina registered_tauri_commands
```

Expected: PASS.

- [ ] **Step 6: Commit Tauri command changes**

Run:

```bash
git add apps/puffer-desktop/src-tauri
git commit -m "feat(desktop): stage chat attachments locally"
```

## Task 4: Frontend API And Types

**Files:**
- Modify: `apps/puffer-desktop/src/lib/types.ts`
- Modify: `apps/puffer-desktop/src/lib/api/desktop.ts`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/attachments.ts`
- Modify: `apps/puffer-desktop/src/lib/agentTurnAttachments.ts`

- [ ] **Step 1: Add TypeScript attachment types**

In `apps/puffer-desktop/src/lib/types.ts`, change attachment types to:

```ts
export type AgentTurnAttachmentKind = "image" | "file";
export type AttachmentState = "available" | "missing";

export type AgentTurnAttachment = {
  id: string;
  name: string;
  mimeType: string;
  size: number;
  extension: string;
  kind: AgentTurnAttachmentKind;
};

export type MessageAttachment = AgentTurnAttachment & {
  state?: AttachmentState;
  previewUrl?: string | null;
};

export type AttachmentPreviewResult =
  | { state: "available"; mimeType: string; bytes: number[] }
  | { state: "missing" }
  | { state: "unsupported" };
```

- [ ] **Step 2: Add API wrappers**

In `apps/puffer-desktop/src/lib/api/desktop.ts`, add:

```ts
type AttachmentStageTestWindow = Window & {
  __PUFFER_TEST_STAGE_CHAT_ATTACHMENT__?: (
    sessionId: string,
    attachment: MessageAttachment & { file?: File }
  ) => Promise<MessageAttachment> | MessageAttachment;
};

export async function stageChatAttachment(
  sessionId: string,
  attachment: MessageAttachment & { file?: File }
): Promise<MessageAttachment> {
  const testStager =
    import.meta.env.DEV && typeof window !== "undefined"
      ? (window as AttachmentStageTestWindow).__PUFFER_TEST_STAGE_CHAT_ATTACHMENT__
      : undefined;
  if (testStager) return testStager(sessionId, attachment);

  if (!canInvokeTauri()) {
    throw new Error("Desktop attachment staging requires the Tauri shell.");
  }
  const file = attachment.file;
  if (!file) throw new Error(`Attachment ${attachment.name} is missing file data.`);
  const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
  return invoke<MessageAttachment>("stage_chat_attachment", {
    sessionId,
    name: attachment.name,
    mimeType: attachment.mimeType,
    extension: attachment.extension,
    kind: attachment.kind,
    bytes
  });
}

export async function readChatAttachmentPreview(
  sessionId: string,
  attachmentId: string
): Promise<AttachmentPreviewResult> {
  if (canInvokeTauri()) {
    return invoke<AttachmentPreviewResult>("read_chat_attachment_preview", {
      sessionId,
      attachmentId
    });
  }
  const client = await ensureLocalDaemonClient();
  return client.request<AttachmentPreviewResult>("read_chat_attachment_preview", {
    sessionId,
    attachmentId
  });
}
```

Import `AttachmentPreviewResult` from `../types`.

- [ ] **Step 3: Change turn options to IDs**

In `apps/puffer-desktop/src/lib/api/desktop.ts`, change:

```ts
export type AgentTurnOptions = {
  providerId?: string | null;
  modelId?: string | null;
  thinkingOptionId?: string | null;
  fastMode?: boolean;
  permissionMode?: AgentPermissionMode;
  mode?: AgentTurnMode;
  attachmentIds?: string[];
};
```

Remove the old `attachments?: AgentTurnAttachment[]` option. Keep `displayAttachments?: MessageAttachment[]` on `AgentTurnSubmitOptions` for optimistic rendering only.

- [ ] **Step 4: Preserve draft file data**

In `apps/puffer-desktop/src/lib/screens/agent/attachments.ts`, ensure `ComposerAttachmentDraft` keeps `file: File` and `messageAttachmentFromDraft` carries it for staging:

```ts
export type ComposerAttachmentDraft = AgentTurnAttachment & {
  file: File;
  previewUrl?: string;
};
```

Update `messageAttachmentFromDraft`:

```ts
export function messageAttachmentFromDraft(attachment: ComposerAttachmentDraft): MessageAttachment & { file: File } {
  return {
    ...attachmentPayloadFromDraft(attachment),
    file: attachment.file,
    previewUrl: attachment.previewUrl ?? null,
    state: "available"
  };
}
```

- [ ] **Step 5: Run frontend typecheck**

Run:

```bash
npm --prefix apps/puffer-desktop run check
```

Expected: FAIL because `handleSubmitMessage` still passes removed attachment metadata through `AgentTurnOptions`, and refreshed attachment preview reads are not wired yet.

Do not commit this task until Task 5 makes typecheck pass.

## Task 5: Submit Flow And Chat Rendering

**Files:**
- Modify: `apps/puffer-desktop/src/App.svelte`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/AttachmentPreviewStrip.svelte`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`
- Modify: `apps/puffer-desktop/src/lib/chatOpenIntent.ts`

- [ ] **Step 1: Stage attachments before submit**

In `apps/puffer-desktop/src/App.svelte`, import `stageChatAttachment` and change `handleSubmitMessage` so it stages drafts before building `turnMessage`:

```ts
const draftAttachments = options.displayAttachments ?? [];
let stagedAttachments: MessageAttachment[] = [];
try {
  stagedAttachments = [];
  for (const attachment of draftAttachments) {
    stagedAttachments = [
      ...stagedAttachments,
      await stageChatAttachment(submitSessionId, attachment as MessageAttachment & { file?: File })
    ];
  }
} catch (error) {
  statusMessage = `Attachment upload failed: ${errorText(error)}`;
  setSubmitMessageInFlight(submitSessionId, false);
  return false;
}

const turnMessage = formatAgentTurnMessage(message, stagedAttachments);
const attachmentIds = stagedAttachments.map((attachment) => attachment.id);
```

Pass `attachmentIds` into `runAgentTurn`:

```ts
const turnId = await runAgentTurn(submitSessionId, turnMessage, {
  ...turnOptions,
  attachmentIds
});
```

Use `stagedAttachments` for the optimistic `TimelineItem.attachments`.

- [ ] **Step 2: Keep remote attachment behavior explicit**

Before staging, reject remote sessions:

```ts
if (remoteConnection.enabled && draftAttachments.length > 0) {
  statusMessage = "File attachments are only supported for local desktop sessions.";
  setSubmitMessageInFlight(submitSessionId, false);
  return false;
}
```

- [ ] **Step 3: Route refreshed attachment preview through API**

In `ConversationView.svelte`, when handling a message attachment open intent:

```ts
async function openMessageAttachment(attachment: MessageAttachment) {
  if (attachment.previewUrl) {
    onOpenChatIntent?.(attachmentOpenIntent(attachment));
    return;
  }
  if (!session?.id) {
    onOpenChatIntent?.(attachmentOpenIntent({ ...attachment, previewUrl: null }));
    return;
  }
  const result = await readChatAttachmentPreview(session.id, attachment.id);
  if (result.state !== "available") {
    onOpenChatIntent?.(attachmentOpenIntent({ ...attachment, previewUrl: null }));
    return;
  }
  const blob = new Blob([new Uint8Array(result.bytes)], { type: result.mimeType });
  const previewUrl = URL.createObjectURL(blob);
  onOpenChatIntent?.(attachmentOpenIntent({ ...attachment, previewUrl }));
}
```

Add cleanup in the overlay close path so preview URLs created from `readChatAttachmentPreview` are revoked.

- [ ] **Step 4: Preserve unavailable detail behavior**

In `AttachmentPreviewStrip.svelte`, keep message cards as buttons. For `state === "missing"`, clicking should emit the attachment intent with no preview URL. The existing overlay should show unavailable detail; do not add a separate disabled card style unless tests require accessible state text.

- [ ] **Step 5: Run frontend typecheck**

Run:

```bash
npm --prefix apps/puffer-desktop run check
```

Expected: PASS.

- [ ] **Step 6: Commit frontend submit/render changes**

Run:

```bash
git add apps/puffer-desktop/src
git commit -m "feat(desktop): reload persisted attachment cards"
```

## Task 6: Desktop Backend Timeline Parity

**Files:**
- Modify: `apps/puffer-desktop/src-tauri/src/backend.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/session_data.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/session_api.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/dtos.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/dto.rs`

- [ ] **Step 1: Add fallback timeline attachment fields**

Update both Tauri timeline enums so `UserMessage` includes:

```rust
attachments: Vec<ChatAttachmentDto>,
```

For in-memory fake `StoredEvent::User` paths in `backend.rs`, use `attachments: Vec::new()` unless that stored event type is also extended in the same task.

- [ ] **Step 2: Map session-store events with attachments**

In `session_data.rs` and `session_api.rs`, when matching:

```rust
TranscriptEvent::UserMessage { text, attachments, actor } => { ... }
```

map `attachments` into `ChatAttachmentDto` using the same `attachment_state` logic from the CLI desktop API.

- [ ] **Step 3: Run Tauri tests**

Run:

```bash
cargo test -p corbina
```

Expected: PASS after all `UserMessage` DTO constructors include `attachments`.

- [ ] **Step 4: Commit Tauri timeline changes**

Run:

```bash
git add apps/puffer-desktop/src-tauri
git commit -m "feat(desktop): return attachment timeline state"
```

## Task 7: Playwright Coverage

**Files:**
- Modify: `apps/puffer-desktop/tests/support/fakeDaemon.ts`
- Modify: `apps/puffer-desktop/tests/chat-session-ui.spec.ts`

- [ ] **Step 1: Extend fake daemon preview response**

Before changing preview responses, add a small test stager helper near the top
of `chat-session-ui.spec.ts` so existing send-with-attachment tests can run in
Vite without pretending production daemon JSON-RPC accepts file bytes:

```ts
async function installAttachmentStageHook(page: Page): Promise<void> {
  await page.addInitScript(() => {
    let nextAttachment = 0;
    (
      window as unknown as {
        __PUFFER_TEST_STAGE_CHAT_ATTACHMENT__?: (
          sessionId: string,
          attachment: Record<string, unknown>
        ) => Record<string, unknown>;
      }
    ).__PUFFER_TEST_STAGE_CHAT_ATTACHMENT__ = (_sessionId, attachment) => {
      nextAttachment += 1;
      return {
        id: String(attachment.id ?? `staged-${nextAttachment}`),
        name: String(attachment.name ?? "attachment"),
        mimeType: String(attachment.mimeType ?? "application/octet-stream"),
        size: Number(attachment.size ?? 0),
        extension: String(attachment.extension ?? "FILE"),
        kind: attachment.kind === "image" ? "image" : "file",
        state: "available"
      };
    };
  });
}
```

Call `await installAttachmentStageHook(page);` before `daemon.install(page)` in
existing tests that submit composer attachments.

Update existing `run_agent_turn` expectations in `chat-session-ui.spec.ts` from
`candidate.params.attachments.length === 2` to:

```ts
Array.isArray(candidate.params.attachmentIds) &&
candidate.params.attachmentIds.length === 2
```

Remove assertions that expect full attachment metadata in `request.params`.
Those refs now live in the session store and timeline DTOs, not the turn RPC.

In `fakeDaemon.ts`, add a preview map:

```ts
private attachmentPreviews = new Map<string, JsonRecord>();

seedAttachmentPreview(sessionId: string, attachmentId: string, preview: JsonRecord): void {
  this.attachmentPreviews.set(`${sessionId}:${attachmentId}`, preview);
}
```

In `dispatch`, add:

```ts
case "read_chat_attachment_preview":
  return this.readAttachmentPreview(request.params);
```

Add:

```ts
private readAttachmentPreview(params: JsonRecord): JsonRecord {
  const sessionId = String(params.sessionId ?? "");
  const attachmentId = String(params.attachmentId ?? "");
  return this.attachmentPreviews.get(`${sessionId}:${attachmentId}`) ?? { state: "missing" };
}
```

- [ ] **Step 2: Add hard-refresh card test**

In `chat-session-ui.spec.ts`, add:

```ts
test("persisted attachment cards survive browser refresh", async ({ page }) => {
  const daemon = new FakeDaemon({
    sessions: [
      {
        sessionId: "session-persisted-attachments",
        displayName: "Persisted attachments",
        title: "Persisted attachments",
        cwd: "/tmp/puffer",
        folderPath: "/tmp/puffer",
        updatedAtMs: baseTime,
        createdAtMs: baseTime - 60_000,
        eventCount: 1,
        timeline: [
          {
            kind: "user_message",
            id: "persisted-user",
            text: "[Image: 1.jpg]",
            createdAtMs: baseTime,
            attachments: [
              {
                id: "att-1",
                name: "1.jpg",
                mimeType: "image/jpeg",
                size: 123,
                extension: "JPG",
                kind: "image",
                state: "available"
              }
            ]
          }
        ]
      }
    ]
  });
  await daemon.install(page);
  await daemon.open(page);
  await openSession(page, /Persisted attachments/);

  await expect(page.getByRole("button", { name: "Open image attachment 1.jpg" })).toBeVisible();
  await expect(page.getByText("[Image: 1.jpg]")).toHaveCount(0);

  await page.reload();
  await openSession(page, /Persisted attachments/);
  await expect(page.getByRole("button", { name: "Open image attachment 1.jpg" })).toBeVisible();
  await expect(page.getByText("[Image: 1.jpg]")).toHaveCount(0);
});
```

- [ ] **Step 3: Add available preview test**

Add:

```ts
test("persisted available image attachment opens preview after refresh", async ({ page }) => {
  const daemon = new FakeDaemon({
    sessions: [
      {
        sessionId: "session-preview-attachment",
        displayName: "Preview attachment",
        title: "Preview attachment",
        cwd: "/tmp/puffer",
        folderPath: "/tmp/puffer",
        updatedAtMs: baseTime,
        createdAtMs: baseTime - 60_000,
        eventCount: 1,
        timeline: [
          {
            kind: "user_message",
            id: "preview-user",
            text: "[Image: preview.png]",
            createdAtMs: baseTime,
            attachments: [
              {
                id: "preview-att",
                name: "preview.png",
                mimeType: "image/png",
                size: 68,
                extension: "PNG",
                kind: "image",
                state: "available"
              }
            ]
          }
        ]
      }
    ]
  });
  daemon.seedAttachmentPreview("session-preview-attachment", "preview-att", {
    state: "available",
    mimeType: "image/png",
    bytes: Array.from(Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lzTnGQAAAABJRU5ErkJggg==",
      "base64"
    ))
  });
  await daemon.install(page);
  await daemon.open(page);
  await openSession(page, /Preview attachment/);
  await page.getByRole("button", { name: "Open image attachment preview.png" }).click();
  await expect(page.getByRole("dialog", { name: "preview.png" })).toBeVisible();
  await expect(page.getByAltText("preview.png")).toBeVisible();
});
```

- [ ] **Step 4: Add missing attachment test**

Add:

```ts
test("persisted missing image attachment shows unavailable detail", async ({ page }) => {
  const daemon = new FakeDaemon({
    sessions: [
      {
        sessionId: "session-missing-attachment",
        displayName: "Missing attachment",
        title: "Missing attachment",
        cwd: "/tmp/puffer",
        folderPath: "/tmp/puffer",
        updatedAtMs: baseTime,
        createdAtMs: baseTime - 60_000,
        eventCount: 1,
        timeline: [
          {
            kind: "user_message",
            id: "missing-user",
            text: "[Image: missing.png]",
            createdAtMs: baseTime,
            attachments: [
              {
                id: "missing-att",
                name: "missing.png",
                mimeType: "image/png",
                size: 68,
                extension: "PNG",
                kind: "image",
                state: "missing"
              }
            ]
          }
        ]
      }
    ]
  });
  await daemon.install(page);
  await daemon.open(page);
  await openSession(page, /Missing attachment/);
  await page.getByRole("button", { name: "Open image attachment missing.png" }).click();
  const dialog = page.getByRole("dialog", { name: "missing.png" });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("Preview unavailable for this attachment.");
  await expect(page.getByAltText("missing.png")).toHaveCount(0);
});
```

- [ ] **Step 5: Run the focused desktop UI tests**

Run:

```bash
npm --prefix apps/puffer-desktop run test:desktop-ui -- tests/chat-session-ui.spec.ts
```

Expected: PASS.

- [ ] **Step 6: Commit Playwright coverage**

Run:

```bash
git add apps/puffer-desktop/tests
git commit -m "test(desktop): cover persisted attachment cards"
```

## Task 8: Final Verification

**Files:**
- No new code files beyond previous tasks.

- [ ] **Step 1: Run Rust verification**

Run:

```bash
cargo test -p puffer-session-store
cargo test -p puffer-cli
cargo test -p corbina
```

Expected: PASS.

- [ ] **Step 2: Run frontend verification**

Run:

```bash
npm --prefix apps/puffer-desktop run check
npm --prefix apps/puffer-desktop run test:desktop-ui -- tests/chat-session-ui.spec.ts
```

Expected: PASS.

- [ ] **Step 3: Review changed files**

Run:

```bash
git status --short
git diff --stat
git diff --check
```

Expected: no whitespace errors from `git diff --check`; only files from this plan are modified.

- [ ] **Step 4: Final commit if verification edits remain**

If `git diff --stat` shows remaining verification fixes, commit them:

```bash
git add crates/puffer-session-store crates/puffer-cli apps/puffer-desktop
git commit -m "fix(desktop): stabilize attachment persistence"
```

If there are no remaining edits, do not create an empty commit.
