# Imagegen Assistant Attachment Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generated `/image` results persist as assistant image attachments and render as the same thumbnail UI used for uploaded image attachments, without showing local paths or metadata in chat.

**Architecture:** Extend the existing session attachment pipeline instead of creating generated-media UI. `AssistantMessage` becomes attachment-capable, generated files are copied into session attachment storage from disk, both daemon media-generation paths append an empty assistant message with the image attachment, and the frontend renders attachments on assistant messages through the existing preview strip.

**Tech Stack:** Rust workspace (`puffer-session-store`, `puffer-cli`, Tauri crate `corbina`), Svelte/TypeScript desktop UI, Playwright desktop UI tests.

**Spec:** `docs/superpowers/specs/2026-06-06-imagegen-assistant-attachment-preview-design.md`

---

## Scope Check

This is one subsystem: durable generated-image chat preview. It intentionally excludes artifact cards, media settings redesign, video previews, thumbnail caches, and file-reveal actions.

## File Structure

- Modify `crates/puffer-session-store/src/events.rs`: allow assistant transcript messages to carry `StoredAttachment` values.
- Modify `crates/puffer-session-store/src/attachments.rs`: add a path-based staging helper that copies generated image files into attachment storage without routing bytes through the frontend.
- Modify `crates/puffer-session-store/src/lib.rs`: export the path-staging input and attachment byte cap for backend callers.
- Modify `apps/puffer-desktop/src-tauri/src/dtos.rs` and `apps/puffer-desktop/src-tauri/src/dto.rs`: expose assistant attachments in desktop timeline DTOs.
- Modify `apps/puffer-desktop/src-tauri/src/session_api.rs` and `apps/puffer-desktop/src-tauri/src/session_data.rs`: map assistant transcript attachments to timeline DTOs.
- Modify `crates/puffer-cli/src/desktop_api_types.rs` and `crates/puffer-cli/src/desktop_api.rs`: map assistant attachments in the CLI daemon desktop API surface.
- Modify `crates/puffer-cli/src/daemon.rs`: make `sessionId` required for image generation, stage the generated file as an attachment, append the assistant transcript event, and stop returning visible paths.
- Modify `apps/puffer-desktop/src-tauri/src/backend.rs`: mirror the same media-generation session write behavior in the Tauri preview backend.
- Modify `apps/puffer-desktop/src/lib/api/desktop.ts`: require `sessionId` in `GenerateMediaInput`, normalize assistant attachments, and stop treating `path` as part of the frontend contract.
- Modify `apps/puffer-desktop/src/App.svelte`: refresh the selected session after `/image` succeeds.
- Modify `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`: render attachment strips for assistant messages.
- Modify `apps/puffer-desktop/src/lib/screens/agent/AttachmentPreviewStrip.svelte`: render missing image attachments as same-size unavailable thumbnails instead of file cards.
- Modify `apps/puffer-desktop/tests/support/fakeDaemon.ts` and `apps/puffer-desktop/tests/chat-session-ui.spec.ts`: cover generated-image preview behavior.

---

### Task 1: Session Store Supports Assistant Attachments

**Files:**
- Modify: `crates/puffer-session-store/src/events.rs`
- Modify: `crates/puffer-session-store/src/attachments.rs`
- Modify: `crates/puffer-session-store/src/lib.rs`

- [ ] **Step 1: Write failing assistant attachment serialization test**

Add this test in `crates/puffer-session-store/src/events.rs` near `user_message_serializes_stored_attachments`:

```rust
#[test]
fn assistant_message_serializes_stored_attachments() {
    let event = TranscriptEvent::AssistantMessage {
        text: String::new(),
        attachments: vec![StoredAttachment {
            id: "att-1".to_string(),
            name: "generated.png".to_string(),
            mime_type: "image/png".to_string(),
            size: 3,
            extension: "PNG".to_string(),
            kind: StoredAttachmentKind::Image,
            storage_key: "att-1/original".to_string(),
        }],
        actor: None,
    };

    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(value["type"], "assistant_message");
    assert_eq!(value["text"], "");
    assert_eq!(value["attachments"][0]["kind"], "image");

    let roundtrip: TranscriptEvent = serde_json::from_value(value).unwrap();
    assert_eq!(roundtrip, event);
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run:

```bash
cargo test -p puffer-session-store assistant_message_serializes_stored_attachments
```

Expected: compile failure because `TranscriptEvent::AssistantMessage` does not have an `attachments` field.

- [ ] **Step 3: Add assistant attachments to the transcript event**

Change the `AssistantMessage` variant in `crates/puffer-session-store/src/events.rs` to:

```rust
AssistantMessage {
    text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<StoredAttachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor: Option<MessageActor>,
},
```

Update existing construction sites in this crate to include
`attachments: Vec::new()`. Update pattern matches that do not read attachments
to use `TranscriptEvent::AssistantMessage { text, actor, .. }`.

- [ ] **Step 4: Write failing path-based staging tests**

Add tests in `crates/puffer-session-store/src/attachments.rs`:

```rust
#[test]
fn stage_attachment_from_file_copies_original_and_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(temp.path());
    let store = SessionStore::from_paths(&paths).unwrap();
    let session = store.create_session(temp.path().to_path_buf()).unwrap();
    let source = temp.path().join("generated.png");
    std::fs::write(&source, [1_u8, 2, 3]).unwrap();

    let attachment = store
        .stage_attachment_from_file(
            session.id,
            StageAttachmentFileInput {
                name: "generated.png".to_string(),
                mime_type: "image/png".to_string(),
                extension: "PNG".to_string(),
                kind: crate::StoredAttachmentKind::Image,
                path: source.clone(),
                max_bytes: Some(MAX_ATTACHMENT_BYTES),
            },
        )
        .unwrap();

    assert_eq!(attachment.name, "generated.png");
    assert_eq!(attachment.size, 3);
    assert_eq!(
        std::fs::read(store.attachment_original_path(session.id, &attachment)).unwrap(),
        vec![1, 2, 3]
    );
    assert_eq!(std::fs::read(source).unwrap(), vec![1, 2, 3]);
}

#[test]
fn stage_attachment_from_file_rejects_oversized_source() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(temp.path());
    let store = SessionStore::from_paths(&paths).unwrap();
    let session = store.create_session(temp.path().to_path_buf()).unwrap();
    let source = temp.path().join("large.png");
    std::fs::write(&source, vec![0_u8; 4]).unwrap();

    let error = store
        .stage_attachment_from_file(
            session.id,
            StageAttachmentFileInput {
                name: "large.png".to_string(),
                mime_type: "image/png".to_string(),
                extension: "PNG".to_string(),
                kind: crate::StoredAttachmentKind::Image,
                path: source,
                max_bytes: Some(3),
            },
        )
        .unwrap_err();

    assert!(error.to_string().contains("attachments must be 3 bytes or smaller"));
}
```

The `attachments.rs` test module already imports `puffer_config::ConfigPaths`;
keep using that import.

- [ ] **Step 5: Run the staging tests and verify they fail**

Run:

```bash
cargo test -p puffer-session-store stage_attachment_from_file
```

Expected: compile failure because `StageAttachmentFileInput`, `MAX_ATTACHMENT_BYTES`, and `stage_attachment_from_file` do not exist.

- [ ] **Step 6: Implement path-based staging**

In `crates/puffer-session-store/src/attachments.rs`, add:

```rust
/// Maximum bytes accepted for one stored chat attachment.
pub const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;

/// Input used when staging one attachment from an existing local file.
#[derive(Debug, Clone)]
pub struct StageAttachmentFileInput {
    /// Original display filename.
    pub name: String,
    /// MIME type associated with the source file.
    pub mime_type: String,
    /// Uppercase display extension.
    pub extension: String,
    /// Attachment kind.
    pub kind: crate::StoredAttachmentKind,
    /// Existing source path to copy into session storage.
    pub path: PathBuf,
    /// Optional maximum byte limit.
    pub max_bytes: Option<u64>,
}
```

Add this method to `impl SessionStore`:

```rust
/// Stages one attachment by copying an existing file into session storage.
pub fn stage_attachment_from_file(
    &self,
    session_id: Uuid,
    input: StageAttachmentFileInput,
) -> Result<StoredAttachment> {
    let metadata = std::fs::metadata(&input.path)
        .with_context(|| format!("failed to stat attachment source {}", input.path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("attachment source must be a file");
    }
    let size = metadata.len();
    if let Some(max_bytes) = input.max_bytes {
        if size > max_bytes {
            anyhow::bail!("attachments must be {max_bytes} bytes or smaller");
        }
    }

    let id = Uuid::new_v4().to_string();
    let dir = self.attachment_dir(session_id, &id)?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create attachment dir {}", dir.display()))?;

    let original_tmp = dir.join("original.tmp");
    let original = dir.join("original");
    std::fs::copy(&input.path, &original_tmp).with_context(|| {
        format!(
            "failed to copy attachment {} to {}",
            input.path.display(),
            original_tmp.display()
        )
    })?;
    std::fs::rename(&original_tmp, &original)
        .with_context(|| format!("failed to stage attachment {}", original.display()))?;

    let attachment = StoredAttachment {
        id: id.clone(),
        name: sanitize_attachment_name(&input.name),
        mime_type: input.mime_type,
        size,
        extension: input.extension,
        kind: input.kind,
        storage_key: format!("{id}/original"),
    };
    let metadata_tmp = dir.join("metadata.json.tmp");
    let metadata_path = dir.join("metadata.json");
    std::fs::write(&metadata_tmp, serde_json::to_vec(&attachment)?).with_context(|| {
        format!(
            "failed to write attachment metadata {}",
            metadata_tmp.display()
        )
    })?;
    std::fs::rename(&metadata_tmp, &metadata_path).with_context(|| {
        format!("failed to stage attachment metadata {}", metadata_path.display())
    })?;
    Ok(attachment)
}
```

Update `crates/puffer-session-store/src/lib.rs` export:

```rust
pub use attachments::{
    AttachmentPreviewBytes, AttachmentState, StageAttachmentFileInput, StageAttachmentInput,
    MAX_ATTACHMENT_BYTES,
};
```

- [ ] **Step 7: Run session-store tests**

Run:

```bash
cargo test -p puffer-session-store assistant_message_serializes_stored_attachments stage_attachment_from_file
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/puffer-session-store/src/events.rs crates/puffer-session-store/src/attachments.rs crates/puffer-session-store/src/lib.rs
git commit -m "feat(session-store): support assistant attachments"
```

---

### Task 2: Timeline DTOs Expose Assistant Attachments

**Files:**
- Modify: `apps/puffer-desktop/src-tauri/src/dtos.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/dto.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/session_api.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/session_data.rs`
- Modify: `crates/puffer-cli/src/desktop_api_types.rs`
- Modify: `crates/puffer-cli/src/desktop_api.rs`

- [ ] **Step 1: Write failing Tauri DTO mapping test**

In `apps/puffer-desktop/src-tauri/src/session_data.rs`, update the test module
imports:

```rust
use super::{parse_tool_message, permission_state, summarize_tool_input, timeline_items};
use crate::dtos::TimelineItemDto;
use puffer_config::ConfigPaths;
use puffer_session_store::{SessionStore, TranscriptEvent};
```

Add this test in that same test module:

```rust
#[test]
fn assistant_messages_return_attachment_dtos() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(temp.path());
    let store = SessionStore::from_paths(&paths).unwrap();
    let session = store.create_session(temp.path().to_path_buf()).unwrap();
    let source = temp.path().join("generated.png");
    std::fs::write(&source, [1_u8, 2, 3]).unwrap();
    let attachment = store
        .stage_attachment_from_file(
            session.id,
            puffer_session_store::StageAttachmentFileInput {
                name: "generated.png".to_string(),
                mime_type: "image/png".to_string(),
                extension: "PNG".to_string(),
                kind: puffer_session_store::StoredAttachmentKind::Image,
                path: source,
                max_bytes: Some(puffer_session_store::MAX_ATTACHMENT_BYTES),
            },
        )
        .unwrap();
    store
        .append_event(
            session.id,
            TranscriptEvent::AssistantMessage {
                text: String::new(),
                attachments: vec![attachment],
                actor: None,
            },
        )
        .unwrap();

    let record = store.load_session(session.id).unwrap();
    let items = timeline_items(&store, &record);
    let assistant = items
        .iter()
        .find(|item| matches!(item, TimelineItemDto::AssistantMessage { .. }))
        .unwrap();
    let TimelineItemDto::AssistantMessage { attachments, text, .. } = assistant else {
        unreachable!("matched assistant message");
    };
    assert_eq!(text, "");
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].name, "generated.png");
    assert_eq!(attachments[0].state, "available");
}
```

- [ ] **Step 2: Write failing CLI DTO mapping test**

In `crates/puffer-cli/src/desktop_api.rs`, add this test beside
`timeline_user_message_includes_attachment_state`:

```rust
#[test]
fn timeline_assistant_message_includes_attachment_state() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::discover(temp.path());
    let store = SessionStore::from_paths(&paths).unwrap();
    let session = store.create_session(temp.path().to_path_buf()).unwrap();
    let source = temp.path().join("generated.png");
    std::fs::write(&source, [1_u8, 2, 3]).unwrap();
    let attachment = store
        .stage_attachment_from_file(
            session.id,
            puffer_session_store::StageAttachmentFileInput {
                name: "generated.png".to_string(),
                mime_type: "image/png".to_string(),
                extension: "PNG".to_string(),
                kind: puffer_session_store::StoredAttachmentKind::Image,
                path: source,
                max_bytes: Some(puffer_session_store::MAX_ATTACHMENT_BYTES),
            },
        )
        .unwrap();
    store
        .append_event(
            session.id,
            TranscriptEvent::AssistantMessage {
                text: String::new(),
                attachments: vec![attachment],
                actor: None,
            },
        )
        .unwrap();

    let detail = load_session_detail(&store, &session.id.to_string()).unwrap();
    let assistant = detail
        .timeline
        .iter()
        .find(|item| matches!(item, TimelineItemDto::AssistantMessage { .. }))
        .unwrap();

    match assistant {
        TimelineItemDto::AssistantMessage {
            attachments, text, ..
        } => {
            assert_eq!(text, "");
            assert_eq!(attachments.len(), 1);
            assert_eq!(attachments[0].name, "generated.png");
            assert_eq!(attachments[0].state, "available");
        }
        _ => unreachable!(),
    }
}
```

- [ ] **Step 3: Run the DTO tests and verify they fail**

Run:

```bash
cargo test -p corbina assistant_messages_return_attachment_dtos
cargo test -p puffer-cli timeline_assistant_message_includes_attachment_state
```

Expected: compile failure because assistant DTO variants do not expose `attachments`.

- [ ] **Step 4: Update DTO enum variants**

In both Tauri DTO files and the CLI desktop API DTO file, change assistant message DTOs to include attachment DTOs:

```rust
AssistantMessage {
    id: String,
    text: String,
    attachments: Vec<ChatAttachmentDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actor: Option<MessageActor>,
},
```

For DTOs that include `created_at_ms`, preserve it and add `attachments` next to `text`.

- [ ] **Step 5: Update transcript-to-timeline mapping**

Every match on assistant messages should bind attachments:

```rust
TranscriptEvent::AssistantMessage {
    text,
    attachments,
    actor,
} => {
    pending_assistant = Some(TimelineItemDto::AssistantMessage {
        id,
        text: text.clone(),
        attachments: attachment_dtos(store, session_id, attachments),
        actor: actor.clone(),
    });
}
```

Use the existing `attachment_dtos` helper already used by user messages.

- [ ] **Step 6: Update construction sites**

For assistant DTOs that are not backed by transcript attachments, pass `attachments: Vec::new()`.

Run:

```bash
rg -n "AssistantMessage \\{" apps/puffer-desktop/src-tauri/src crates/puffer-cli/src
```

Expected: every assistant DTO construction includes `attachments`.

- [ ] **Step 7: Run DTO tests**

Run:

```bash
cargo test -p corbina assistant_messages_return_attachment_dtos
cargo test -p puffer-cli timeline_assistant_message_includes_attachment_state
```

Expected: both pass.

- [ ] **Step 8: Commit**

```bash
git add apps/puffer-desktop/src-tauri/src/dtos.rs apps/puffer-desktop/src-tauri/src/dto.rs apps/puffer-desktop/src-tauri/src/session_api.rs apps/puffer-desktop/src-tauri/src/session_data.rs apps/puffer-desktop/src-tauri/src/backend.rs crates/puffer-cli/src/desktop_api_types.rs crates/puffer-cli/src/desktop_api.rs
git commit -m "feat(desktop): expose assistant attachments in timelines"
```

---

### Task 3: CLI Daemon Persists Generated Images As Assistant Attachments

**Files:**
- Modify: `crates/puffer-cli/src/daemon.rs`

- [ ] **Step 1: Write failing missing-session test**

Add near existing `generate_media_*` tests:

```rust
#[test]
fn generate_media_requires_session_id_before_provider_execution() {
    let state = test_daemon_state_with_image_config();
    let error = handle_generate_media(
        &state,
        &json!({
            "kind": "image",
            "prompt": "draw an icon"
        }),
    )
    .unwrap_err();

    assert!(error.to_string().contains("missing sessionId"));
}
```

Add a small local helper for this test module if one is not already present:

```rust
fn temp_daemon_state() -> (PufferHomeEnvGuard, tempfile::TempDir, DaemonState) {
    let home_guard = PufferHomeEnvGuard::set();
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspace");
    let paths = ConfigPaths {
        workspace_root: workspace_root.clone(),
        workspace_config_dir: workspace_root.join(".puffer"),
        user_config_dir: temp.path().join("home").join(".puffer"),
        builtin_resources_dir: workspace_root.join("resources"),
    };
    ensure_workspace_dirs(&paths).expect("workspace dirs");
    let state = DaemonState::load(workspace_root, paths, "token".into(), true, false, false)
        .expect("daemon state");
    (home_guard, temp, state)
}
```

Use it in the missing-session test:

```rust
let (_home_guard, _temp, state) = temp_daemon_state();
```

- [ ] **Step 2: Write failing assistant-attachment persistence test**

Add a test beside `generate_media_returns_succeeded_image_artifact_metadata`:

```rust
#[test]
fn generate_media_appends_assistant_image_attachment_without_path_text() {
    let (state, session_id) = test_daemon_state_with_image_config_and_session();
    let result = handle_generate_media(
        &state,
        &json!({
            "sessionId": session_id.to_string(),
            "kind": "image",
            "prompt": "draw an icon"
        }),
    )
    .unwrap();

    assert!(result.get("path").is_none());
    let store = puffer_session_store::SessionStore::from_paths(&state.paths).unwrap();
    let record = store.load_session(session_id).unwrap();
    let assistant = record
        .events
        .iter()
        .find_map(|event| match event {
            puffer_session_store::TranscriptEvent::AssistantMessage {
                text,
                attachments,
                ..
            } => Some((text, attachments)),
            _ => None,
        })
        .unwrap();
    assert_eq!(assistant.0, "");
    assert_eq!(assistant.1.len(), 1);
    assert_eq!(assistant.1[0].kind, puffer_session_store::StoredAttachmentKind::Image);
    assert!(!record.events.iter().any(|event| {
        matches!(
            event,
            puffer_session_store::TranscriptEvent::AssistantMessage { text, .. }
                if text.contains("/.puffer/workflows/images/")
        )
    }));
}
```

Before invoking `handle_generate_media`, create the session explicitly:

```rust
let session_store = puffer_session_store::SessionStore::from_paths(&state.paths).unwrap();
let session = session_store.create_session(state.paths.workspace_root.clone()).unwrap();
let session_id = session.id;
```

- [ ] **Step 3: Run CLI daemon tests and verify they fail**

Run:

```bash
cargo test -p puffer-cli generate_media_requires_session_id_before_provider_execution generate_media_appends_assistant_image_attachment_without_path_text
```

Expected: compile or assertion failure because `GenerateMediaParams` does not require `sessionId`, `path` is returned, and no assistant transcript event is appended.

- [ ] **Step 4: Update CLI generate media params and result**

In `crates/puffer-cli/src/daemon.rs`:

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateMediaParams {
    session_id: String,
    kind: String,
    prompt: String,
}
```

Remove `path: Option<String>` from `GenerateMediaResult`.

- [ ] **Step 5: Add helper functions**

Add helpers near `generate_image_media_job`:

```rust
fn generated_image_name(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("generated-image.png")
        .to_string()
}

fn generated_image_extension(path: &std::path::Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.trim().is_empty())
        .map(|extension| extension.to_ascii_uppercase())
        .unwrap_or_else(|| "PNG".to_string())
}

fn generated_image_mime_type(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg".to_string(),
        Some("webp") => "image/webp".to_string(),
        _ => "image/png".to_string(),
    }
}
```

- [ ] **Step 6: Append assistant attachment after generation**

Change `handle_generate_media` image branch to pass `input.session_id`:

```rust
match kind.as_str() {
    "image" => generate_image_media_job(state, input.session_id, prompt),
    "video" => generate_video_media_job(state, prompt),
    _ => unreachable!("media kind was validated"),
}
```

Change the image function signature and body:

```rust
fn generate_image_media_job(
    state: &DaemonState,
    session_id: String,
    prompt: String,
) -> Result<Value> {
    let session_uuid = Uuid::parse_str(&session_id).context("invalid sessionId")?;
    let session_store = SessionStore::from_paths(&state.paths)?;
    session_store
        .load_session(session_uuid)
        .with_context(|| format!("unknown session `{session_id}`"))?;

    // keep the existing config, capability, and generate_exact_image_with_cache logic

    let attachment = session_store.stage_attachment_from_file(
        session_uuid,
        puffer_session_store::StageAttachmentFileInput {
            name: generated_image_name(&generation.path),
            mime_type: generated_image_mime_type(&generation.path),
            extension: generated_image_extension(&generation.path),
            kind: puffer_session_store::StoredAttachmentKind::Image,
            path: generation.path.clone(),
            max_bytes: Some(puffer_session_store::MAX_ATTACHMENT_BYTES),
        },
    )?;
    session_store.append_event(
        session_uuid,
        TranscriptEvent::AssistantMessage {
            text: String::new(),
            attachments: vec![attachment],
            actor: None,
        },
    )?;
    state.publish_event(ServerEnvelope::Event {
        event: "workspace:sessions:changed".to_string(),
        payload: json!({
            "reason": "media_generation",
            "sessionId": session_id,
        }),
    });

    let result = GenerateMediaResult {
        job_id: generation.job_id,
        artifact_id: Some(generation.artifact_id),
        kind: "image".to_string(),
        provider_id: generation.provider_id,
        model_id: generation.model_id,
        status: generation.status,
        prompt,
    };
    Ok(serde_json::to_value(result)?)
}
```

Use the existing imported `Uuid`, `SessionStore`, `TranscriptEvent`, `json!`, and `Context` names where already available.

- [ ] **Step 7: Run CLI daemon tests**

Run:

```bash
cargo test -p puffer-cli generate_media
```

Expected: all `generate_media` tests pass after updating existing tests to pass `sessionId` and to expect no `path` field.

- [ ] **Step 8: Commit**

```bash
git add crates/puffer-cli/src/daemon.rs
git commit -m "feat(daemon): persist generated images as assistant attachments"
```

---

### Task 4: Tauri Preview Backend Mirrors Generated Image Persistence

**Files:**
- Modify: `apps/puffer-desktop/src-tauri/src/backend.rs`

- [ ] **Step 1: Write failing Tauri backend tests**

Add tests beside existing `generate_media_*` tests:

```rust
#[test]
fn generate_media_requires_session_id_before_tauri_provider_execution() {
    let backend = BackendState::new();
    let error = backend
        .generate_media(json!({
            "kind": "image",
            "prompt": "draw an icon"
        }))
        .unwrap_err();
    assert!(error.to_string().contains("missing sessionId"));
}

#[test]
fn generate_media_appends_tauri_assistant_image_attachment_without_path_text() {
    let backend = BackendState::new();
    let store = backend.session_store().unwrap();
    let session = store.create_session(backend.default_workspace().unwrap()).unwrap();

    let result = backend
        .generate_media(json!({
            "sessionId": session.id.to_string(),
            "kind": "image",
            "prompt": "draw an icon"
        }))
        .unwrap();

    assert!(serde_json::to_value(&result).unwrap().get("path").is_none());
    let record = store.load_session(session.id).unwrap();
    let assistant = record
        .events
        .iter()
        .find_map(|event| match event {
            puffer_session_store::TranscriptEvent::AssistantMessage {
                text,
                attachments,
                ..
            } => Some((text, attachments)),
            _ => None,
        })
        .unwrap();
    assert_eq!(assistant.0, "");
    assert_eq!(assistant.1.len(), 1);
}
```

Call the backend through its RPC dispatcher so private helper visibility does
not matter:

```rust
let result = backend
    .handle(
        crate::events::EventEmitter::websocket_only(),
        "generate_media",
        json!({
            "sessionId": session.id.to_string(),
            "kind": "image",
            "prompt": "draw an icon"
        }),
    )
    .unwrap();
```

- [ ] **Step 2: Run Tauri backend tests and verify they fail**

Run:

```bash
cargo test -p corbina generate_media_requires_session_id_before_tauri_provider_execution generate_media_appends_tauri_assistant_image_attachment_without_path_text
```

Expected: compile or assertion failure for missing session persistence.

- [ ] **Step 3: Update params/result and add session store helper**

In `apps/puffer-desktop/src-tauri/src/backend.rs`, change `GenerateMediaParams`:

```rust
struct GenerateMediaParams {
    session_id: String,
    kind: String,
    prompt: String,
}
```

Remove `path` from `GenerateMediaResult`.

Add a backend helper if one does not already exist:

```rust
fn session_store(&self) -> Result<SessionStore> {
    SessionStore::from_paths(&ConfigPaths::discover(self.default_workspace()?))
}
```

- [ ] **Step 4: Mirror CLI append logic**

Update `generate_image_media_job` to accept `session_id: String`, validate/load the session before provider execution, then after `generate_exact_image_with_cache` call `stage_attachment_from_file` and append:

```rust
let attachment = session_store.stage_attachment_from_file(
    session_uuid,
    puffer_session_store::StageAttachmentFileInput {
        name: generated_image_name(&generation.path),
        mime_type: generated_image_mime_type(&generation.path),
        extension: generated_image_extension(&generation.path),
        kind: puffer_session_store::StoredAttachmentKind::Image,
        path: generation.path.clone(),
        max_bytes: Some(puffer_session_store::MAX_ATTACHMENT_BYTES),
    },
)?;
session_store.append_event(
    session_uuid,
    puffer_session_store::TranscriptEvent::AssistantMessage {
        text: String::new(),
        attachments: vec![attachment],
        actor: None,
    },
)?;
```

Use the same `generated_image_name`, `generated_image_extension`, and `generated_image_mime_type` helpers from Task 3. Duplicating these three tiny pure helpers in the Tauri backend is acceptable here; do not create a shared media-attachment crate for this change.

- [ ] **Step 5: Run Tauri backend tests**

Run:

```bash
cargo test -p corbina generate_media
```

Expected: all Tauri `generate_media` tests pass after updating existing tests to pass `sessionId` and to expect no `path`.

- [ ] **Step 6: Commit**

```bash
git add apps/puffer-desktop/src-tauri/src/backend.rs
git commit -m "feat(desktop): persist generated image previews in tauri backend"
```

---

### Task 5: Frontend Renders Assistant Image Attachments

**Files:**
- Modify: `apps/puffer-desktop/src/lib/types.ts`
- Modify: `apps/puffer-desktop/src/lib/api/desktop.ts`
- Modify: `apps/puffer-desktop/src/App.svelte`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/AttachmentPreviewStrip.svelte`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/MessageAttachmentPreviewStrip.svelte`

- [ ] **Step 1: Update frontend API types**

In `apps/puffer-desktop/src/lib/api/desktop.ts`, make `sessionId` required and remove `path`:

```ts
export type GenerateMediaInput = {
  sessionId: string;
  kind: MediaKind;
  prompt: string;
};

export type GenerateMediaResult = {
  jobId: string;
  artifactId: string | null;
  kind: MediaKind;
  providerId: string;
  modelId: string;
  status: string;
  prompt: string;
};
```

Extend the assistant backend timeline item:

```ts
| ({
    kind: "assistant_message";
    id: string;
    text: string;
    createdAtMs?: number | null;
    attachments?: BackendChatAttachment[];
  } & BackendActorFields)
```

- [ ] **Step 2: Normalize assistant attachments**

In `normalizeTimelineItem`, update the assistant branch:

```ts
case "assistant_message": {
  const attachments = (value.attachments ?? []).map(normalizeMessageAttachment);
  return {
    id: value.id,
    kind: "assistant",
    createdAtMs: value.createdAtMs ?? null,
    title: "Assistant response",
    summary: preview(value.text),
    body: value.text,
    meta: [],
    ...(attachments.length > 0 ? { attachments } : {}),
    actor: value.actor ?? null
  };
}
```

- [ ] **Step 3: Refresh selected session after `/image` succeeds**

In `apps/puffer-desktop/src/App.svelte`, after successful `generateMedia`, reload the selected session without resetting live state:

```ts
const result = await generateMedia({
  sessionId,
  kind: request.kind,
  prompt: request.prompt
});
if (selectedSession?.id === sessionId) {
  const detail = await loadSessionDetailFromDaemon(sessionId);
  selectedSession = detail.session;
  sessionDetail = {
    ...detail,
    timeline: reuseTransientMessageIds(detail.timeline, [...submittedMessages, ...liveStreamItems])
  };
}
statusMessage = `${request.kind === "image" ? "Image" : "Video"} media job ${result.jobId.slice(0, 8)} ${result.status}.`;
```

Keep the existing attachment rejection behavior for media slash commands.

- [ ] **Step 4: Render assistant attachments**

In `ConversationView.svelte`, inside the assistant/agent message body before text rendering, add:

```svelte
{#if row.item.attachments?.length}
  <MessageAttachmentPreviewStrip
    sessionId={session?.id ?? null}
    attachments={row.item.attachments}
    {onOpenChatIntent}
  />
{/if}
```

Ensure this is applied to the visible assistant message row, not tool activity child rows.

- [ ] **Step 5: Render missing images as image placeholders**

In `AttachmentPreviewStrip.svelte`, change the preview snippet:

```svelte
{#if attachment.kind === "image" && attachment.previewUrl}
  <div class="pf-attachment-thumb">
    <img src={attachment.previewUrl} alt={attachment.name} draggable="false" />
  </div>
{:else if attachment.kind === "image"}
  <div class="pf-attachment-thumb pf-attachment-thumb-missing" aria-label="Image preview unavailable">
    <Icon name="image" size={18} />
  </div>
{:else}
  <div class="pf-attachment-file-card" data-kind={attachment.kind}>
    ...
  </div>
{/if}
```

Add CSS:

```css
.pf-attachment-thumb-missing {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--muted-foreground);
  background: color-mix(in oklab, var(--muted) 58%, var(--background));
}
```

Use the existing `file` icon inside the square thumbnail placeholder. Do not
render the file-card layout for missing image attachments.

- [ ] **Step 6: Run frontend checks**

Run:

```bash
cd apps/puffer-desktop && npm run check
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/puffer-desktop/src/lib/types.ts apps/puffer-desktop/src/lib/api/desktop.ts apps/puffer-desktop/src/App.svelte apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte apps/puffer-desktop/src/lib/screens/agent/AttachmentPreviewStrip.svelte apps/puffer-desktop/src/lib/screens/agent/MessageAttachmentPreviewStrip.svelte
git commit -m "feat(desktop): render generated images as assistant thumbnails"
```

---

### Task 6: Fake Daemon And E2E Coverage

**Files:**
- Modify: `apps/puffer-desktop/tests/support/fakeDaemon.ts`
- Modify: `apps/puffer-desktop/tests/chat-session-ui.spec.ts`

- [ ] **Step 1: Update fake daemon media generation**

In `FakeDaemon.generateMedia`, require `sessionId`, append an assistant timeline item, and seed preview bytes:

```ts
const sessionId = String(params.sessionId ?? "");
if (!sessionId) throw new Error("missing sessionId");
const metadata = this.sessions.get(sessionId);
if (!metadata) throw new Error(`unknown session \`${sessionId}\``);
const attachmentId = `generated-image-${Date.now().toString(36)}`;
const imageBytes = Array.from(Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lzTnGQAAAABJRU5ErkJggg==",
  "base64"
));
this.seedAttachmentPreview(sessionId, attachmentId, {
  state: "available",
  mimeType: "image/png",
  bytes: imageBytes
});
const current = this.timelines.get(sessionId) ?? [];
this.setSessionTimeline(sessionId, [
  ...current,
  {
    kind: "assistant_message",
    id: `generated-image-message-${attachmentId}`,
    text: "",
    createdAtMs: Date.now(),
    attachments: [
      {
        id: attachmentId,
        name: "generated.png",
        mimeType: "image/png",
        kind: "image",
        extension: "PNG",
        size: imageBytes.length,
        state: "available"
      }
    ]
  }
]);
```

Return no `path` field:

```ts
return {
  jobId,
  artifactId: "fixture-artifact",
  kind,
  providerId: settings.providerId,
  modelId: settings.modelId,
  status: "succeeded",
  prompt
};
```

- [ ] **Step 2: Add E2E generated thumbnail assertion**

Extend the `/image` slash trigger test in `chat-session-ui.spec.ts`:

```ts
const generatedImageButton = page.getByRole("button", {
  name: "Open image attachment generated.png"
});
await expect(generatedImageButton).toBeVisible();
await expect(generatedImageButton.getByAltText("generated.png")).toBeVisible();
await expect(page.getByText("/Users/zhangxiao/.puffer/workflows/images/modern-airplane.png")).toHaveCount(0);
await expect(page.getByText("generated.png")).toHaveCount(0);
```

The button accessible name may include the filename because the shared attachment UI uses it for accessibility. The visible chat body must not show the filename.

- [ ] **Step 3: Add missing generated image E2E**

Add a focused test:

```ts
test("generated image missing file renders thumbnail placeholder without path", async ({ page }) => {
  const daemon = new FakeDaemon({
    sessions: [
      {
        sessionId: "session-generated-missing",
        displayName: "Generated missing",
        title: "Generated missing",
        cwd: "/tmp/puffer",
        folderPath: "/tmp/puffer",
        updatedAtMs: baseTime,
        createdAtMs: baseTime - 60_000,
        eventCount: 1,
        timeline: [
          {
            kind: "assistant_message",
            id: "generated-missing-message",
            text: "",
            createdAtMs: baseTime - 30_000,
            attachments: [
              {
                id: "generated-missing-image",
                name: "modern-airplane.png",
                mimeType: "image/png",
                kind: "image",
                extension: "PNG",
                size: 100,
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
  await openSession(page, /Generated missing/);

  const button = page.getByRole("button", { name: "Open image attachment modern-airplane.png" });
  await expect(button).toBeVisible();
  await expect(page.getByText("modern-airplane.png")).toHaveCount(0);
  await expect(page.getByText("/Users/zhangxiao/.puffer/workflows/images/modern-airplane.png")).toHaveCount(0);
  await button.click();
  const dialog = page.getByRole("dialog", { name: "modern-airplane.png" });
  await expect(dialog).toContainText("Preview unavailable");
});
```

- [ ] **Step 4: Run E2E tests**

Run:

```bash
cd apps/puffer-desktop && npx playwright test tests/chat-session-ui.spec.ts -g "image|generated image"
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/puffer-desktop/tests/support/fakeDaemon.ts apps/puffer-desktop/tests/chat-session-ui.spec.ts
git commit -m "test(desktop): cover generated image thumbnails"
```

---

### Task 7: Final Verification And Spec Sync

**Files:**
- Modify if behavior changed: `docs/superpowers/specs/2026-06-06-imagegen-assistant-attachment-preview-design.md`

- [ ] **Step 1: Run focused Rust tests**

Run:

```bash
cargo test -p puffer-session-store assistant_message_serializes_stored_attachments stage_attachment_from_file
cargo test -p puffer-cli generate_media
cargo test -p corbina generate_media
```

Expected: all pass.

- [ ] **Step 2: Run frontend checks**

Run:

```bash
cd apps/puffer-desktop && npm run check
cd apps/puffer-desktop && npx playwright test tests/chat-session-ui.spec.ts -g "image|generated image"
```

Expected: all pass.

- [ ] **Step 3: Review visible-path regression**

Run:

```bash
rg -n "path:|\\.path|generated\\.path|/\\.puffer/workflows/images" apps/puffer-desktop/src crates/puffer-cli/src apps/puffer-desktop/src-tauri/src
```

Expected: `generation.path` is used only server-side for staging/copying and artifact bookkeeping. Frontend generated-media rendering does not display path text.

- [ ] **Step 4: Confirm spec alignment**

Read:

```text
docs/superpowers/specs/2026-06-06-imagegen-assistant-attachment-preview-design.md
```

Confirm the implementation still matches these requirements:

- thumbnail-only visible chat UI;
- required `sessionId`;
- no path text fallback;
- missing images use same-size unavailable thumbnail placeholder.

- [ ] **Step 5: Final status check**

Run:

```bash
git status --short
```

Expected: no unstaged implementation changes remain.
