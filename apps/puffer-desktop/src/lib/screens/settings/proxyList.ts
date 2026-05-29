import type { NetworkProxySettings } from "../../types";

export function removeProxyEndpoint(
  proxy: NetworkProxySettings,
  proxyId: string
): NetworkProxySettings {
  const proxies = proxy.proxies.filter((item) => item.id !== proxyId);
  const removedSelected = proxy.selected === proxyId;
  const selectedStillExists = Boolean(
    proxy.selected && proxies.some((item) => item.id === proxy.selected)
  );
  let selected = selectedStillExists ? proxy.selected : null;

  if (!selected && proxies.length > 0 && (removedSelected || proxy.enabled)) {
    selected = proxies[0].id;
  }

  return {
    ...proxy,
    enabled: proxies.length > 0 ? proxy.enabled : false,
    selected,
    proxies,
    lastTest: proxy.lastTest?.proxyId === proxyId ? null : proxy.lastTest
  };
}
