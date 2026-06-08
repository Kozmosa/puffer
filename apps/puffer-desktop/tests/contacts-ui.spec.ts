import { expect, test, type Page } from "@playwright/test";
import { FakeDaemon } from "./support/fakeDaemon";

async function openContacts(page: Page) {
  await page.locator(".pf-sidebar").getByRole("button", { name: "Contacts" }).click();
}

test("contacts tab saves a user-curated contact", async ({ page }) => {
  const daemon = new FakeDaemon();
  await daemon.install(page);
  await daemon.open(page);

  await openContacts(page);
  await daemon.waitForRequest("contacts_list");

  const form = page.locator(".pf-contact-form");
  await form.getByLabel("Name").fill("Launch Alice");
  await form.getByLabel("Description").fill("Alice asks high-signal launch and support questions.");
  await form.getByLabel("Contact IDs").fill("telegram@alice\ngoogle@alice@example.com");
  await form.getByRole("button", { name: "Create" }).click();

  const request = await daemon.waitForRequest("contacts_save");
  expect(request.params.contact_ids).toEqual(["telegram@alice", "google@alice@example.com"]);
  await expect(page.locator(".pf-contact-card").filter({ hasText: "Launch Alice" })).toBeVisible();
});
