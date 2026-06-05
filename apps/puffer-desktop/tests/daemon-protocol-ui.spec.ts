import { expect, type Page, test } from "@playwright/test";
import { FakeDaemon } from "./support/fakeDaemon";

async function openRegressionAgent(page: Page): Promise<void> {
  await page
    .locator(".pf-sidebar-agents-list")
    .getByRole("button", { name: /^Browser regression\b/ })
    .click();
}

function orderIndex(order: string[], needles: string[]): number {
  return order.findIndex((text) => needles.some((needle) => text.includes(needle)));
}

function expectBefore(order: string[], before: string[], after: string[]): void {
  const beforeIndex = orderIndex(order, before);
  const afterIndex = orderIndex(order, after);
  expect(beforeIndex).toBeGreaterThanOrEqual(0);
  expect(afterIndex).toBeGreaterThanOrEqual(0);
  expect(beforeIndex).toBeLessThan(afterIndex);
}

async function mockTauriEnsureLocalDaemon(
  page: Page,
  handshakes: Array<{ url: string; workspaceRoot: string }>
): Promise<void> {
  await page.addInitScript((input) => {
    const win = window as unknown as {
      __TAURI__?: unknown;
      __TAURI_INTERNALS__?: {
        invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
      };
    };
    let index = 0;
    win.__TAURI__ = {};
    win.__TAURI_INTERNALS__ = {
      invoke: async (cmd: string) => {
        if (cmd !== "ensure_local_daemon") throw new Error(`unexpected invoke: ${cmd}`);
        const handshake = input[Math.min(index, input.length - 1)];
        index += 1;
        return {
          url: handshake.url,
          token: "test",
          protocolVersion: "2025-01-01",
          workspaceRoot: handshake.workspaceRoot
        };
      }
    };
  }, handshakes);
}

test("desktop client speaks the real daemon WebSocket protocol", async ({ page }) => {
  const daemon = new FakeDaemon({ protocol: "real" });
  await daemon.install(page);
  await daemon.open(page);

  await expect.poll(() => daemon.socketUrls.length).toBeGreaterThan(0);
  expect(daemon.socketUrls[0]).toContain("token=test");

  await openRegressionAgent(page);
  await page.locator(".pf-composer textarea").fill("Use real daemon framing");
  await page.getByRole("button", { name: "Send" }).click();

  const turnRequest = await daemon.waitForRequest(
    "run_agent_turn",
    (request) => request.params.message === "Use real daemon framing"
  );
  expect(typeof turnRequest.id).toBe("string");
  expect(turnRequest.params).toMatchObject({
    sessionId: "session-browser",
    providerId: "codex",
    modelId: "test-model"
  });

  daemon.setSessionTimeline("session-browser", [
    {
      kind: "user_message",
      id: "real-user",
      text: "Use real daemon framing",
      createdAtMs: Date.now() - 1000
    },
    {
      kind: "assistant_message",
      id: "real-assistant",
      text: "Real daemon completion arrived.",
      createdAtMs: Date.now()
    }
  ]);
  daemon.emit("session:session-browser:event", {
    type: "turn-complete",
    turnId: "turn-session-browser",
    assistantText: "Real daemon completion arrived."
  });

  await expect(page.getByText("Real daemon completion arrived.")).toBeVisible();
});

test("streams intermediate assistant messages inside agent activity", async ({ page }) => {
  const daemon = new FakeDaemon({ protocol: "real" });
  daemon.setSessionTimeline("session-browser", []);
  await daemon.install(page);
  await daemon.open(page);

  await openRegressionAgent(page);
  await page.locator(".pf-composer textarea").fill("Interleave live activity");
  await page.getByRole("button", { name: "Send" }).click();

  await daemon.waitForRequest(
    "run_agent_turn",
    (request) => request.params.message === "Interleave live activity"
  );
  const turnId = "turn-session-browser";
  const channel = "session:session-browser:event";
  daemon.emit(channel, { type: "turn-start", turnId });
  daemon.emit(channel, {
    type: "text-delta",
    turnId,
    delta: "First intermediate."
  });
  daemon.emit(channel, {
    type: "tool-calls-requested",
    turnId,
    requests: [
      {
        callId: "call-read",
        toolId: "Read",
        input: "{\"path\":\"README.md\"}"
      }
    ]
  });
  daemon.emit(channel, {
    type: "tool-invocations",
    turnId,
    invocations: [
      {
        callId: "call-read",
        toolId: "Read",
        input: "{\"path\":\"README.md\"}",
        output: "{\"content\":\"read output\"}",
        success: true
      }
    ]
  });
  daemon.emit(channel, {
    type: "text-delta",
    turnId,
    delta: "Second intermediate."
  });
  daemon.emit(channel, {
    type: "tool-calls-requested",
    turnId,
    requests: [
      {
        callId: "call-bash",
        toolId: "Bash",
        input: "{\"command\":\"npm test\"}"
      }
    ]
  });
  daemon.emit(channel, {
    type: "tool-invocations",
    turnId,
    invocations: [
      {
        callId: "call-bash",
        toolId: "Bash",
        input: "{\"command\":\"npm test\"}",
        output: "{\"stdout\":\"ok\"}",
        success: true
      }
    ]
  });
  daemon.emit(channel, {
    type: "text-delta",
    turnId,
    delta: "Final answer."
  });

  await expect(
    page.locator(".agent-tools .activity-message").filter({ hasText: "First intermediate." })
  ).toBeVisible();
  await expect(
    page.locator(".agent-tools .activity-message").filter({ hasText: "Second intermediate." })
  ).toBeVisible();
  const runningOrder = await page
    .locator(".agent-tools .activity-message, .agent-tools .pf-tool")
    .evaluateAll((nodes) =>
      nodes.map((node) => node.textContent?.replace(/\s+/g, " ").trim() ?? "")
    );
  expectBefore(runningOrder, ["First intermediate."], ["Read"]);
  expectBefore(runningOrder, ["Read"], ["Second intermediate."]);
  expectBefore(runningOrder, ["Second intermediate."], ["Bash", "Shell", "npm test"]);

  const completedAtMs = Date.now();
  daemon.setSessionTimeline("session-browser", [
    {
      kind: "user_message",
      id: "interleaved-user",
      text: "Interleave live activity",
      createdAtMs: completedAtMs - 4000
    },
    {
      kind: "tool_call",
      id: "persisted-read",
      toolId: "Read",
      status: "success",
      inputText: "{\"path\":\"README.md\"}",
      inputJson: { path: "README.md" },
      outputText: "{\"content\":\"read output\"}",
      createdAtMs: completedAtMs - 3000
    },
    {
      kind: "tool_call",
      id: "persisted-bash",
      toolId: "Bash",
      status: "success",
      inputText: "{\"command\":\"npm test\"}",
      inputJson: { command: "npm test" },
      outputText: "{\"stdout\":\"ok\"}",
      createdAtMs: completedAtMs - 2000
    },
    {
      kind: "assistant_message",
      id: "persisted-final",
      text: "Final answer.",
      createdAtMs: completedAtMs - 1000
    }
  ]);
  daemon.emit(channel, {
    type: "turn-complete",
    turnId,
    assistantText: "Final answer."
  });

  await expect(page.getByText("Final answer.")).toBeVisible();
  const activity = page.locator(".activity-group").filter({ hasText: "Agent activity" });
  await expect(activity).toBeVisible();
  const activityButton = activity.getByRole("button", { name: /Agent activity/ });
  await expect(activityButton).toHaveAttribute("aria-expanded", "false");
  await activityButton.click();
  await expect(
    activity.locator(".activity-message").filter({ hasText: "First intermediate." })
  ).toBeVisible();
  await expect(
    activity.locator(".activity-message").filter({ hasText: "Second intermediate." })
  ).toBeVisible();
  await expect(
    activity.locator(".activity-message").filter({ hasText: "Final answer." })
  ).toHaveCount(0);
  const foldedOrder = await activity
    .locator(".activity-details > .activity-message, .activity-details > .activity-action")
    .evaluateAll((nodes) =>
      nodes.map((node) => node.textContent?.replace(/\s+/g, " ").trim() ?? "")
    );
  expectBefore(foldedOrder, ["First intermediate."], ["Read"]);
  expectBefore(foldedOrder, ["Read"], ["Second intermediate."]);
  expectBefore(foldedOrder, ["Second intermediate."], ["Bash", "Shell", "npm test"]);
});

test("backend reconnect button reports failed retries", async ({ page }) => {
  const daemon = new FakeDaemon({ protocol: "real" });
  await daemon.install(page);
  await daemon.open(page);

  await expect.poll(() => daemon.socketUrls.length).toBeGreaterThan(0);
  await daemon.dropConnections();

  const banner = page.locator(".connection-banner");
  await expect(banner).toContainText("Puffer backend disconnected.");

  await banner.getByRole("button", { name: "Reconnect backend" }).click();
  await expect(banner).toContainText("Reconnect failed:");
  await expect(banner).toContainText("Unable to connect to Puffer daemon");

  daemon.allowConnections();
  await banner.getByRole("button", { name: "Reconnect backend" }).click();

  await expect.poll(() => daemon.socketUrls.length).toBeGreaterThan(1);
  await expect(page.locator(".connection-banner")).toHaveCount(0);
});

test("changed daemon port reconnect reacquires the native local daemon", async ({ page }) => {
  const oldDaemon = new FakeDaemon({
    protocol: "real",
    url: "ws://127.0.0.1:17777/ws",
    workspaceRoot: "/tmp/puffer-old-port"
  });
  const newDaemon = new FakeDaemon({
    protocol: "real",
    url: "ws://127.0.0.1:17778/ws",
    workspaceRoot: "/tmp/puffer-new-port"
  });
  await mockTauriEnsureLocalDaemon(page, [
    { url: oldDaemon.url, workspaceRoot: "/tmp/puffer-old-port" },
    { url: newDaemon.url, workspaceRoot: "/tmp/puffer-new-port" }
  ]);
  await oldDaemon.install(page);
  await newDaemon.install(page);

  await page.goto("/?skipOnboarding=1");
  await expect.poll(() => oldDaemon.socketUrls.length).toBeGreaterThan(0);

  await oldDaemon.dropConnections();
  const banner = page.locator(".connection-banner");
  await expect(banner).toContainText("Puffer backend disconnected.");

  await banner.getByRole("button", { name: "Reconnect backend" }).click();

  await expect.poll(() => newDaemon.socketUrls.length).toBeGreaterThan(0);
  await expect(page.locator(".connection-banner")).toHaveCount(0);
});

test("native daemon acquisition ignores stale browser-configured local backend", async ({
  page
}) => {
  const staleDaemon = new FakeDaemon({
    protocol: "real",
    url: "ws://127.0.0.1:17779/ws",
    workspaceRoot: "/tmp/puffer-stale-browser"
  });
  const nativeDaemon = new FakeDaemon({
    protocol: "real",
    url: "ws://127.0.0.1:17780/ws",
    workspaceRoot: "/tmp/puffer-native"
  });
  await page.addInitScript((staleUrl) => {
    window.localStorage.setItem("puffer.backendUrl", staleUrl);
    window.localStorage.setItem("puffer.backendToken", "test");
  }, staleDaemon.url);
  await mockTauriEnsureLocalDaemon(page, [
    { url: nativeDaemon.url, workspaceRoot: "/tmp/puffer-native" }
  ]);
  await staleDaemon.install(page);
  await nativeDaemon.install(page);

  await page.goto("/?skipOnboarding=1");

  await expect.poll(() => nativeDaemon.socketUrls.length).toBeGreaterThan(0);
  expect(staleDaemon.socketUrls).toHaveLength(0);
  await expect(page.locator(".connection-banner")).toHaveCount(0);
});
