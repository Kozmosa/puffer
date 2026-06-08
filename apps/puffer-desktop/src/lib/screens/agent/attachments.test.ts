import { expect, test } from "vitest";
import { addAttachmentFiles, messageAttachmentFromDraft } from "./attachments";

test("composer attachment drafts do not fabricate durable source metadata", () => {
  const file = new File(["# notes"], "notes.md", { type: "text/markdown" });

  const result = addAttachmentFiles({
    current: [],
    files: [file],
    nextId: () => "draft-1"
  });

  expect(result.error).toBeNull();
  expect(result.attachments).toHaveLength(1);
  expect("source" in result.attachments[0]).toBe(false);
});

test("message attachments created from drafts keep file data without a source", () => {
  const file = new File(["# notes"], "notes.md", { type: "text/markdown" });
  const draft = addAttachmentFiles({
    current: [],
    files: [file],
    nextId: () => "draft-1"
  }).attachments[0];

  const upload = messageAttachmentFromDraft(draft);

  expect(upload.file).toBe(file);
  expect(upload.name).toBe("notes.md");
  expect(upload.previewUrl).toBeNull();
  expect("source" in upload).toBe(false);
});
