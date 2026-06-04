import assert from "node:assert/strict";
import fs from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const testDir = dirname(fileURLToPath(import.meta.url));
const configPath = resolve(testDir, "../src-tauri/tauri.conf.json");
const config = JSON.parse(fs.readFileSync(configPath, "utf8"));
const windows = config?.app?.windows;

assert.ok(Array.isArray(windows), "tauri.conf.json app.windows must be an array");

const mainWindow = windows.find((windowConfig) => windowConfig.title === "Corbina");

assert.ok(mainWindow, "tauri.conf.json must define the main Corbina window");
assert.equal(
  mainWindow.dragDropEnabled,
  false,
  "main window must disable Tauri native drag/drop so HTML5 file drops reach the frontend"
);
