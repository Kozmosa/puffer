# Chat Composer Autosize Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the primary Corbina Chat composer textarea grow with multi-line prompts, cap at 200px, and collapse after clearing or sending.

**Architecture:** Keep autosize local to `ConversationView.svelte`. Use the native textarea `scrollHeight` pattern from agent-sdk, but do not extract a shared component, add a dependency, or apply the behavior to other textareas. Preserve existing draft, attachment, submit, keyboard, provider, and permission contracts.

**Tech Stack:** Svelte 5 runes, native `<textarea>`, existing `chat.css`, Playwright, existing desktop fake daemon.

---

## Scope And Guardrails

Implement only `docs/superpowers/specs/2026-06-04-chat-composer-autosize-design.md`.

Do not touch mini window, Ask Puffer, legacy `ConversationPane`, settings textareas, file editor textareas, workflow textareas, provider routing, attachment payloads, submit API shape, IME keyboard behavior, or fake daemon behavior.

Do not add a textarea autosize package, Svelte action, extracted component, hidden mirror element, observer, debounce, or browser `field-sizing` dependency.

## File Structure

- Modify `apps/puffer-desktop/tests/chat-session-ui.spec.ts`
  - Adds one focused Playwright regression test for Chat composer height growth, capped overflow, session draft restoration, and collapse after send.

- Modify `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`
  - Owns the local textarea ref, height measurement functions, and explicit call sites for draft changes.

- Modify `apps/puffer-desktop/src/lib/design/chat.css`
  - Keeps current textarea styling and adds the base overflow state.

- Create `specs/puffer-desktop/663.md`
  - Documents the desktop component behavior and compatibility implications.

## Task 1: Add the Failing Autosize Regression Test

**Files:**
- Modify: `apps/puffer-desktop/tests/chat-session-ui.spec.ts`

- [ ] **Step 1: Add a textarea metrics helper**

In `apps/puffer-desktop/tests/chat-session-ui.spec.ts`, add this helper after `dispatchFileDrag`:

```ts
async function composerTextareaMetrics(page: Page): Promise<{
  height: number;
  scrollHeight: number;
  clientHeight: number;
  overflowY: string;
}> {
  return page.locator(".pf-composer textarea").evaluate((node) => {
    const textarea = node as HTMLTextAreaElement;
    const rect = textarea.getBoundingClientRect();
    return {
      height: rect.height,
      scrollHeight: textarea.scrollHeight,
      clientHeight: textarea.clientHeight,
      overflowY: getComputedStyle(textarea).overflowY
    };
  });
}
```

- [ ] **Step 2: Add the failing test**

Add this test near the other composer tests in `apps/puffer-desktop/tests/chat-session-ui.spec.ts`:

```ts
test("composer textarea autosizes long drafts and resets after send", async ({ page }) => {
  const daemon = new FakeDaemon({
    sessions: [
      {
        sessionId: "session-autosize-alpha",
        displayName: "Autosize alpha",
        title: "Autosize alpha",
        cwd: "/tmp/puffer",
        folderPath: "/tmp/puffer",
        updatedAtMs: baseTime,
        createdAtMs: baseTime - 60_000,
        eventCount: 1,
        timeline: [
          {
            kind: "assistant_message",
            id: "autosize-alpha-seed",
            text: "Use the composer for a long prompt.",
            createdAtMs: baseTime - 30_000
          }
        ]
      },
      {
        sessionId: "session-autosize-beta",
        displayName: "Autosize beta",
        title: "Autosize beta",
        cwd: "/tmp/puffer",
        folderPath: "/tmp/puffer",
        updatedAtMs: baseTime - 1_000,
        createdAtMs: baseTime - 55_000,
        eventCount: 1,
        timeline: [
          {
            kind: "assistant_message",
            id: "autosize-beta-seed",
            text: "Second session for draft restoration.",
            createdAtMs: baseTime - 25_000
          }
        ]
      }
    ]
  });

  await daemon.install(page);
  await daemon.open(page);

  await openSession(page, /Autosize alpha/);
  const composer = page.locator(".pf-composer textarea");
  await expect(composer).toBeEnabled();

  const initialMetrics = await composerTextareaMetrics(page);
  const multilinePrompt = [
    "Audit the current chat composer.",
    "Keep the implementation local.",
    "Preserve Enter and Shift+Enter behavior.",
    "Avoid shared abstractions.",
    "Add a focused regression test.",
    "Report any risks."
  ].join("\n");

  await composer.fill(multilinePrompt);
  await expect(composer).toHaveValue(multilinePrompt);
  await expect
    .poll(async () => (await composerTextareaMetrics(page)).height)
    .toBeGreaterThan(initialMetrics.height + 24);

  const grownMetrics = await composerTextareaMetrics(page);

  await openSession(page, /Autosize beta/);
  await expect(composer).toHaveValue("");

  await openSession(page, /Autosize alpha/);
  await expect(composer).toHaveValue(multilinePrompt);
  await expect
    .poll(async () => (await composerTextareaMetrics(page)).height)
    .toBeGreaterThanOrEqual(grownMetrics.height - 1);

  const longPrompt = Array.from(
    { length: 40 },
    (_, index) => `long composer line ${index + 1}`
  ).join("\n");

  await composer.fill(longPrompt);
  await expect(composer).toHaveValue(longPrompt);
  await expect
    .poll(async () => (await composerTextareaMetrics(page)).overflowY)
    .toBe("auto");

  const cappedMetrics = await composerTextareaMetrics(page);
  expect(cappedMetrics.height).toBeGreaterThan(160);
  expect(cappedMetrics.height).toBeLessThanOrEqual(205);
  expect(cappedMetrics.scrollHeight).toBeGreaterThan(cappedMetrics.clientHeight);
  expect(cappedMetrics.overflowY).toBe("auto");

  await page.getByRole("button", { name: "Send" }).click();
  await daemon.waitForRequest(
    "run_agent_turn",
    (request) =>
      request.params.sessionId === "session-autosize-alpha" &&
      request.params.message === longPrompt
  );
  await expect(composer).toHaveValue("");
  await expect
    .poll(async () => (await composerTextareaMetrics(page)).height)
    .toBeLessThanOrEqual(initialMetrics.height + 4);
});
```

- [ ] **Step 3: Run the focused test and verify it fails**

Run:

```bash
cd apps/puffer-desktop
npm run test:desktop -- tests/chat-session-ui.spec.ts -g "composer textarea autosizes long drafts and resets after send"
```

Expected: FAIL before implementation because the textarea height does not grow by more than 24px.

## Task 2: Implement Local Textarea Autosize

**Files:**
- Modify: `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`
- Modify: `apps/puffer-desktop/src/lib/design/chat.css`

- [ ] **Step 1: Add the local max-height constant**

In `ConversationView.svelte`, add this constant near the existing top-level constants:

```ts
const COMPOSER_MAX_HEIGHT_PX = 200;
```

The nearby block should become:

```ts
const ENGINEER_NAME = "Engineer";
const RECAP_DISPLAY_PREFIX = "\u203B recap: ";
const COMPOSER_MAX_HEIGHT_PX = 200;
```

- [ ] **Step 2: Add the textarea ref**

In the local state block that already declares `fileInputEl`, add:

```ts
let composerTextareaEl: HTMLTextAreaElement | undefined;
```

The nearby declarations should include:

```ts
let fileInputEl: HTMLInputElement | undefined;
let composerTextareaEl: HTMLTextAreaElement | undefined;
let attachmentMenuEl: HTMLDivElement | undefined;
let threadEl: HTMLDivElement | undefined;
```

- [ ] **Step 3: Add the resize helpers**

Add these functions before `updateDraft`:

```ts
function resizeComposerTextarea(textarea: HTMLTextAreaElement | undefined = composerTextareaEl) {
  if (!textarea) return;
  textarea.style.height = "auto";
  const scrollHeight = textarea.scrollHeight;
  const nextHeight = Math.min(scrollHeight, COMPOSER_MAX_HEIGHT_PX);
  textarea.style.height = `${nextHeight}px`;
  textarea.style.overflowY = scrollHeight > COMPOSER_MAX_HEIGHT_PX ? "auto" : "hidden";
}

function scheduleComposerResize() {
  void tick().then(() => resizeComposerTextarea());
}
```

- [ ] **Step 4: Schedule resize through the existing draft update path**

Replace `updateDraft` with:

```ts
function updateDraft(value: string) {
  draft = value;
  setDraftForSession(session?.id, value);
  scheduleComposerResize();
}
```

- [ ] **Step 5: Schedule resize on session draft restoration**

In the session-switching `$effect`, add `scheduleComposerResize();` after `lastSessionId = nextSessionId;`.

The end of that block should read:

```ts
      selectedActivityChildren = {};
      lastSessionId = nextSessionId;
      scheduleComposerResize();
      void tick().then(() => threadEl?.scrollTo({ top: 0, behavior: "auto" }));
```

- [ ] **Step 6: Schedule resize on submit clear and restore paths**

In `submit()`, after `clearCurrentAttachments();`, add:

```ts
    scheduleComposerResize();
```

In the `accepted === false` branch, after the attachment restoration block and before `return;`, add:

```ts
        scheduleComposerResize();
```

In the `catch` block, after the attachment restoration block, add:

```ts
      scheduleComposerResize();
```

- [ ] **Step 7: Bind the textarea element**

Update the composer textarea markup in `ConversationView.svelte` from:

```svelte
<textarea
  value={draft}
  placeholder={session ? `Reply to ${engineerName}…` : "Select a session to continue"}
  oninput={(event) => updateDraft(event.currentTarget.value)}
  onkeydown={onKeydown}
  disabled={composerDisabled}
></textarea>
```

to:

```svelte
<textarea
  bind:this={composerTextareaEl}
  value={draft}
  placeholder={session ? `Reply to ${engineerName}…` : "Select a session to continue"}
  oninput={(event) => updateDraft(event.currentTarget.value)}
  onkeydown={onKeydown}
  disabled={composerDisabled}
></textarea>
```

- [ ] **Step 8: Add the base overflow CSS**

In `apps/puffer-desktop/src/lib/design/chat.css`, update the textarea rule from:

```css
.pf-composer textarea {
  width: 100%; border: 0; outline: 0; background: transparent;
  resize: none; min-height: 26px; max-height: 200px;
  font-size: var(--pf-chat-text-size); font-family: var(--font-sans); line-height: 1.5;
  color: var(--foreground);
  padding: 4px 4px 6px;
}
```

to:

```css
.pf-composer textarea {
  width: 100%; border: 0; outline: 0; background: transparent;
  resize: none; min-height: 26px; max-height: 200px; overflow-y: hidden;
  font-size: var(--pf-chat-text-size); font-family: var(--font-sans); line-height: 1.5;
  color: var(--foreground);
  padding: 4px 4px 6px;
}
```

- [ ] **Step 9: Run the focused test and verify it passes**

Run:

```bash
cd apps/puffer-desktop
npm run test:desktop -- tests/chat-session-ui.spec.ts -g "composer textarea autosizes long drafts and resets after send"
```

Expected: PASS.

## Task 3: Add the Desktop Component Update Spec

**Files:**
- Create: `specs/puffer-desktop/663.md`

- [ ] **Step 1: Create the update spec**

Create `specs/puffer-desktop/663.md` with this content:

```md
# puffer-desktop Update 663: Chat Composer Autosize

## Summary

The primary agent Chat composer textarea now grows with multi-line draft
content, caps at the existing 200px composer height, and scrolls internally for
very long prompts.

## Architecture

Autosize is implemented locally in `ConversationView.svelte` with a native
textarea ref and `scrollHeight` measurement. The implementation does not add a
shared component, Svelte action, new package dependency, hidden mirror element,
observer, or CSS `field-sizing` dependency.

## Behavior

- Empty and short drafts keep the compact composer height.
- Multi-line drafts grow until the 200px cap.
- Very long drafts keep the composer capped and scroll inside the textarea.
- Per-session draft restoration re-measures the composer height.
- Accepted sends clear the draft and collapse the composer.
- Rejected or failed sends restore the draft and re-measure the composer height.

## Contracts

Submit payloads, attachment payloads, model routing controls, permission
controls, Enter-to-send, Shift+Enter newline behavior, and IME composition
handling are unchanged.

## Verification

Playwright coverage in `apps/puffer-desktop/tests/chat-session-ui.spec.ts`
asserts height growth, 200px cap behavior, internal overflow, per-session draft
height restoration, and collapse after send.
```

- [ ] **Step 2: Verify the file exists**

Run:

```bash
test -f specs/puffer-desktop/663.md
```

Expected: exit code 0.

## Task 4: Run Focused Verification

**Files:**
- No file edits.

- [ ] **Step 1: Run the autosize regression test**

Run:

```bash
cd apps/puffer-desktop
npm run test:desktop -- tests/chat-session-ui.spec.ts -g "composer textarea autosizes long drafts and resets after send"
```

Expected: PASS.

- [ ] **Step 2: Run adjacent composer behavior tests**

Run:

```bash
cd apps/puffer-desktop
npm run test:desktop -- tests/chat-session-ui.spec.ts -g "composer enter does not submit while IME composition is active|unsent composer drafts are preserved per session while switching|failed turn start keeps composer draft"
```

Expected: PASS. These checks guard the keyboard and draft-recovery paths that autosize now touches.

- [ ] **Step 3: Run Svelte type checking**

Run:

```bash
cd apps/puffer-desktop
npm run check
```

Expected: PASS.

## Task 5: Review And Commit

**Files:**
- Review all modified files.

- [ ] **Step 1: Review the diff**

Run:

```bash
git diff -- apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte apps/puffer-desktop/src/lib/design/chat.css apps/puffer-desktop/tests/chat-session-ui.spec.ts specs/puffer-desktop/663.md
```

Expected: diff only contains the local autosize behavior, focused test, CSS overflow base state, and component update spec.

- [ ] **Step 2: Confirm no unrelated files are staged**

Run:

```bash
git status --short
```

Expected: only the autosize implementation files plus any already-existing unrelated untracked files are shown. Do not stage unrelated untracked files.

- [ ] **Step 3: Commit the implementation**

Run:

```bash
git add apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte apps/puffer-desktop/src/lib/design/chat.css apps/puffer-desktop/tests/chat-session-ui.spec.ts specs/puffer-desktop/663.md
git commit -m "feat(desktop): autosize chat composer"
```

Expected: commit succeeds with only autosize implementation files included.
