import { expect, test } from "@playwright/test";
import type { NetworkProxySettings, SanitizedProxyEndpoint } from "../src/lib/types";
import { removeProxyEndpoint } from "../src/lib/screens/settings/proxyList";

const localProxy: SanitizedProxyEndpoint = {
  id: "local",
  scheme: "socks5",
  host: "127.0.0.1",
  port: 7890,
  username: null,
  hasPassword: false,
  uri: "socks5://127.0.0.1:7890"
};

const backupProxy: SanitizedProxyEndpoint = {
  id: "backup",
  scheme: "socks5h",
  host: "127.0.0.1",
  port: 7891,
  username: "tester",
  hasPassword: true,
  uri: "socks5h://127.0.0.1:7891"
};

function proxySettings(overrides: Partial<NetworkProxySettings> = {}): NetworkProxySettings {
  return {
    enabled: true,
    selected: "local",
    bypass: ["localhost"],
    proxies: [localProxy, backupProxy],
    lastTest: {
      proxyId: "local",
      ok: true,
      message: "Connected",
      latencyMs: 848,
      statusCode: 204
    },
    ...overrides
  };
}

test("remove proxy endpoint selects the next saved proxy", () => {
  const next = removeProxyEndpoint(proxySettings(), "local");

  expect(next.enabled).toBe(true);
  expect(next.selected).toBe("backup");
  expect(next.proxies).toEqual([backupProxy]);
  expect(next.lastTest).toBeNull();
});

test("remove final proxy endpoint disables proxy routing", () => {
  const next = removeProxyEndpoint(
    proxySettings({
      proxies: [localProxy]
    }),
    "local"
  );

  expect(next.enabled).toBe(false);
  expect(next.selected).toBeNull();
  expect(next.proxies).toEqual([]);
});
