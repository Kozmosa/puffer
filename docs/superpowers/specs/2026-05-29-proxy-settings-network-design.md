# Proxy Settings and Network Routing Design

Date: 2026-05-29
Status: reviewed and refined for implementation planning

## Intent

Add first-class proxy configuration to Puffer Desktop and the Puffer daemon so
provider traffic can reliably use a user-selected proxy. The first
implementation must be a small, complete provider-proxy loop: configure proxy,
test proxy, persist proxy, route model/provider network calls through it, and
show actionable failures in chat or Settings.

This revision intentionally narrows the initial scope. Puffer can support an
eventual "all external HTTP" mode, but the first version must not expose UI that
implies unmigrated workflow, connector, MCP, or WebFetch traffic is already
proxied.

## Recheck Findings

- Direct `reqwest` construction is not as broad as the first design assumed for
  provider traffic. The important paths are concentrated in model send helpers,
  provider discovery, and provider OAuth helpers.
- OpenAI and Anthropic provider crates mostly build ordered request payloads.
  The model request builders should remain unchanged; proxy support belongs at
  the send/client boundary.
- A broad network target enum for tools, MCP, connectors, and local
  services is premature for phase 1.
- A new shared HTTP crate is premature for phase 1. Existing dependency
  direction allows daemon and CLI code to construct proxy-aware clients through
  `puffer-core` and inject those clients into discovery/OAuth helpers.
- A config `scope` field is premature while there is only one supported
  behavior. Phase 1 has one effective scope: provider traffic.
- A persistent global HTTP client cache is premature. Current code already
  builds short-lived blocking clients in several model paths; the proxy feature
  should first centralize construction and add caching only after profiling or
  repeated-turn latency shows a real need.
- Separate RPC methods for select/delete/edit are unnecessary. The UI can edit
  a full proxy settings draft and persist it through one save API, plus one
  focused test API.
- System proxy import, PAC, rule engines, automatic proxy ranking, and
  failover would expand the feature without helping the current WorldRouter and
  SOCKS5 use case.

## Goals

- Provide a native Settings UI for proxy setup that matches the current Puffer
  Settings layout.
- Support HTTP, HTTPS, SOCKS5, and SOCKS5H proxy endpoints.
- Recommend SOCKS5H for LLM provider traffic so DNS resolution happens through
  the proxy.
- Proxy model streaming, non-streaming provider model calls, provider model
  discovery, and provider OAuth exchange/refresh when proxy is enabled.
- Keep localhost and private-network development endpoints off the proxy by
  default through explicit bypass rules.
- Preserve Anthropic request construction and header-order compatibility. Proxy
  support must not rewrite Anthropic request builders.
- Keep the implementation incremental and testable; do not migrate unrelated
  workflow, connector, MCP, marketplace, or browser call sites in phase 1.

## Non-Goals

- PAC file support.
- Automatic macOS or Windows system proxy import.
- Reliance on `HTTP_PROXY`, `HTTPS_PROXY`, or system proxy environment
  behavior.
- Multi-proxy automatic failover.
- Latency-based proxy ranking.
- Per-provider proxy rules.
- Generic proxy rule engines.
- Phase 1 support for WebFetch, MCP HTTP, workflow HTTP, connectors, plugin
  marketplace, heartbeat, browser Chrome DevTools probes, or subscriber HTTP.
- Backward-compatible behavior for older experimental proxy keys.

## User Experience

Settings adds a new `Network` section in the existing left navigation.

The pane follows the existing `Settings.svelte` grammar:

- simple settings use row layout with title, description, and right-side
  controls;
- lists use section layout with title and description above full-width cards;
- card actions stay inside the card when they operate on that exact item;
- add actions sit below lists and align left;
- modal editing follows the existing Settings modal style.

### Network Pane Structure

1. `Proxy`
   - Master switch.
   - No status badge in this row. Status belongs to individual proxy cards.

2. `Proxy list`
   - Title and description above the list.
   - Shows configured proxies as cards.
   - One proxy can be selected at a time.
   - Each proxy card shows:
     - selected marker or ordinal icon;
     - proxy URI without credentials;
     - last test status, for example `connected - 42 ms`;
     - `Test`;
     - `Edit`.
   - `Add proxy` appears below the list, left aligned.

3. `Bypass`
   - Title and description above the editor.
   - Full-width textarea for comma or newline separated entries.
   - Default entries:
     - `localhost`
     - `127.0.0.1`
     - `::1`
     - `10.0.0.0/8`
     - `172.16.0.0/12`
     - `192.168.0.0/16`
   - Actions:
     - `Reset defaults`
     - `Save bypass`

4. `Connectivity check`
   - Tests the selected proxy against the active provider's discovery endpoint
     when possible.
   - Falls back to the active provider base URL when discovery is unavailable.
   - Uses a short timeout, 5 to 8 seconds.

### Deferred Scope Control

Do not expose an active `All Puffer HTTP` control in phase 1. Showing that
choice before WebFetch, workflow HTTP, MCP HTTP, connectors, and marketplace
call sites are migrated would overpromise behavior.

Do not add a config `scope` field in phase 1. The only supported behavior is
provider traffic. A later phase can add `scope = "all"` at the same time it
migrates and verifies the broader call sites.

### Add/Edit Proxy Modal

Fields:

- Type: `SOCKS5H`, `SOCKS5`, `HTTP`, `HTTPS`
- Host
- Port
- Username, optional
- Password, optional

Actions:

- `Test`
- `Cancel`
- `Save`

The saved list stores credentials in the user config for the first
implementation. The UI must never render passwords back in plain text after
save; it should show a filled password placeholder and only replace the stored
value when the user edits that field.

## Configuration Model

Add a `network` section to `PufferConfig`.

```toml
[network.proxy]
enabled = true
selected = "local-socks"
bypass = [
  "localhost",
  "127.0.0.1",
  "::1",
  "10.0.0.0/8",
  "172.16.0.0/12",
  "192.168.0.0/16",
]

[[network.proxy.proxies]]
id = "local-socks"
scheme = "socks5h"
host = "127.0.0.1"
port = 7890
username = ""
password = ""
```

Rust types in `puffer-config`:

- `NetworkConfig`
- `ProxyConfig`
- `ProxyEndpoint`
- `ProxyScheme`

Validation rules:

- `enabled = true` requires `selected` to reference an existing proxy.
- `host` must be non-empty.
- `port` must be in `1..=65535`.
- `scheme` must be one of `http`, `https`, `socks5`, `socks5h`.
- Proxy IDs must be stable, non-empty, and unique.
- Bypass supports exact hostnames, IP addresses, and CIDR ranges.
- Do not support wildcard, glob, regex, or PAC matching in phase 1.

Config is user-scoped. Desktop proxy save writes to the user config path, not
the workspace config path.

## Network Architecture

Add a small `puffer-core::network` module instead of a new crate. This keeps the
initial implementation inside the existing runtime boundary and avoids a new
workspace package before there is cross-crate behavior broad enough to justify
one.

The module stays narrow and phase-1-only:

- blocking `reqwest::blocking::Client` construction;
- proxy URL construction;
- bypass matching;
- credential redaction helpers;
- test-target request helper for Settings.

Do not add async client construction in phase 1. Async MCP OAuth and streamable
HTTP can move in a later phase if `All Puffer HTTP` is implemented.

Phase 1 purpose enum:

```rust
pub enum HttpPurpose {
    Model,
    Discovery,
    OAuth,
    ConnectivityTest,
}
```

Phase 1 public helpers:

- build a blocking client for a purpose and timeout;
- build a `reqwest::Proxy` from the selected endpoint;
- decide whether a URL matches bypass;
- redact proxy credentials for UI snapshots and logs;
- test one proxy endpoint against one URL with a short timeout.

Provider registry and OAuth crates do not depend on `puffer-core`. They receive
proxy-aware `reqwest::blocking::Client` values through new injection methods
instead:

- `ModelDiscoveryClient::with_client(client)`;
- OpenAI OAuth `_with_client` helpers;
- Anthropic OAuth `_with_client` helpers.

Cargo changes:

- add the `socks` feature to workspace `reqwest`;
- add `ipnet` for CIDR parsing rather than hand-rolling CIDR parsing;
- keep the existing rustls TLS choice in phase 1.

## Routing Semantics

When proxy is enabled and the destination URL does not match bypass, proxy
these calls:

- `Model`: OpenAI/OpenAI-compatible and Anthropic streaming sends.
- `Model`: non-streaming provider sends through `runtime/http_support.rs`.
- `Model`: provider-backed web search/model-tool sends that use OpenAI or
  Anthropic request builders.
- `Discovery`: provider model discovery in `puffer-provider-registry`.
- `OAuth`: OpenAI and Anthropic authorization-code exchange, refresh, and
  profile/role follow-up calls used by login flows.
- `ConnectivityTest`: Settings proxy test requests.

Never proxy in phase 1:

- local browser Chrome DevTools probes;
- local MCP and local tool servers;
- Ollama, LM Studio, ComfyUI, and other localhost services;
- workflow HTTP, WebFetch, connector APIs, marketplace, heartbeat, and
  subscriber HTTP.

Bypass is evaluated before purpose. If the URL matches bypass, it does not use
the proxy even for provider traffic.

## Desktop API

Extend `load_settings_snapshot` with sanitized network proxy state:

- enabled;
- selected proxy ID;
- sanitized proxy list;
- bypass entries;
- last in-memory test result when available.

Add focused daemon RPCs:

- `save_proxy_settings`
- `test_proxy`

Do not add separate select, edit, or delete RPCs in phase 1. The Svelte screen
can update a local draft object and persist the full proxy settings document via
`save_proxy_settings`.

`test_proxy` accepts either a saved proxy ID or a draft endpoint payload, so the
Add/Edit modal can test before saving.

## Error Handling

Proxy errors should identify the failing layer without leaking credentials:

- invalid proxy config;
- selected proxy missing;
- proxy connection failed;
- proxy auth failed;
- target request failed after proxy connected;
- bypass matched, proxy skipped.

UI presentation:

- card-level status for test results;
- request-level error in chat when model traffic fails;
- no global app startup failure for invalid proxy config;
- no password, token, or proxy credential in logs, snapshots, or test payloads.

## Performance

Do not add a global client cache in phase 1.

Reasoning:

- existing model paths already build blocking clients per request;
- LLM latency dominates client construction cost;
- cache invalidation on Settings save adds shared mutable state and risk;
- a centralized builder gives one clean place to add caching later if profiling
  shows it matters.

Phase 1 performance requirements:

- no proxy test blocks app startup;
- proxy test timeout is short;
- provider discovery keeps its current short timeout behavior;
- model request timeout remains compatible with existing long streaming
  requests.

## Testing

Required Rust tests:

- config parse and serialize for `network.proxy`;
- validation failures for invalid scheme, port, selected ID, duplicate ID, and
  invalid bypass entry;
- sanitized snapshot redacts credentials;
- proxy URL builder supports HTTP, HTTPS, SOCKS5, and SOCKS5H;
- bypass matcher handles localhost, IPv4, IPv6, exact hosts, and CIDR;
- builder applies a proxy for provider URLs when enabled;
- builder skips proxy for bypassed local URLs;
- model send helpers use the proxy-aware client builder;
- provider discovery can receive an injected proxy-aware client;
- OpenAI OAuth refresh/exchange can receive an injected proxy-aware client;
- Anthropic OAuth refresh/exchange/profile calls can receive an injected
  proxy-aware client;
- invalid proxy produces an actionable chat error.

Required desktop tests:

- Settings snapshot includes sanitized network proxy data;
- `save_proxy_settings` persists to user config;
- `test_proxy` handles saved proxy ID and draft endpoint payload;
- Settings UI can add, edit, select, test, and save bypass entries;
- password placeholder does not overwrite the stored password unless edited.

Integration test strategy:

- Use a local HTTP target plus a local lightweight HTTP proxy stub to verify
  that provider discovery/model test traffic reaches the proxy.
- Unit-test SOCKS5/SOCKS5H URL construction. Do not build a custom SOCKS server
  in phase 1 unless a lightweight existing test helper is already available.

## Execution Plan

### Step 1: Config Contract

- Add `network` config structs to `puffer-config`.
- Add default bypass list and validation helpers.
- Add serde tests for TOML round-trip and defaults.
- Add redaction helpers for proxy endpoints.

Exit criteria:

- `cargo test -p puffer-config` passes.
- Existing configs without `network` still parse through serde defaults.

### Step 2: Minimal `puffer-core::network`

- Add `crates/puffer-core/network.rs` and export only what daemon/runtime code
  needs.
- Implement blocking client builder for `HttpPurpose`.
- Implement proxy URL construction and bypass matching.
- Implement short-timeout proxy test helper.
- Add unit tests for URL construction, bypass, redaction, and proxy decision.

Exit criteria:

- focused `puffer-core::network` unit tests pass.
- No new crate, async, MCP, connector, workflow, or cache APIs are added.

### Step 3: Provider Discovery Wiring

- Add `ModelDiscoveryClient::with_client(client)`.
- Add registry methods that accept proxy-aware discovery clients where daemon
  and CLI have config available.
- Keep default `ModelDiscoveryClient::new()` for tests and non-proxy callers.

Exit criteria:

- provider discovery tests pass;
- a local proxy integration test proves discovery traffic can be routed through
  the proxy-aware client.

### Step 4: Model Send Wiring

- Replace model-send client construction in:
  - `crates/puffer-core/runtime/openai.rs`;
  - `crates/puffer-core/runtime/anthropic.rs`;
  - `crates/puffer-core/runtime/http_support.rs`;
  - provider-backed `web_search` sends that use model APIs.
- Keep OpenAI and Anthropic request-builder crates unchanged except where OAuth
  client injection is needed.

Exit criteria:

- model runtime tests pass;
- a local proxy integration test proves at least one OpenAI-compatible model
  send reaches the proxy;
- Anthropic request parity tests still pass.

### Step 5: Provider OAuth Wiring

- Add `_with_client` variants for OpenAI and Anthropic OAuth exchange/refresh
  helpers.
- Route daemon and CLI OAuth flows through proxy-aware clients when provider
  proxy is enabled.
- Preserve existing public helper behavior for callers that do not pass a
  client.

Exit criteria:

- provider OAuth unit tests pass;
- daemon `login_with_oauth` and CLI auth refresh paths compile and use the new
  injected client where config is available.

### Step 6: Desktop API

- Extend settings snapshot DTO and frontend types with sanitized proxy state.
- Add `save_proxy_settings` and `test_proxy` daemon RPCs.
- Persist through `save_user_config`.
- Keep `update_config` focused on existing scalar fields.

Exit criteria:

- daemon API tests cover snapshot, save, validation, and test error payloads.
- Existing Settings snapshot consumers continue to render.

### Step 7: Settings UI

- Add `Network` navigation item.
- Add Proxy row, Proxy list section, Add/Edit modal, Bypass section, and
  Connectivity check.
- Keep `Test` and `Edit` inside proxy cards.
- Keep `Add proxy` below the list and left aligned.
- Keep Bypass title/description above the textarea and include `Save bypass`.
- Do not expose active `All Puffer HTTP` scope in phase 1.

Exit criteria:

- UI handles empty, one-proxy, multi-proxy, selected, failed-test, and
  password-placeholder states.
- Local desktop build renders the pane without layout overlap.

### Step 8: Verification Gate

- Run focused package tests for changed crates.
- Run relevant daemon/API tests.
- Run desktop typecheck/build if available in the project scripts.
- Manually verify with a local SOCKS5 or HTTP proxy:
  - enable proxy;
  - select proxy;
  - test proxy;
  - send a WorldRouter/OpenAI-compatible chat message;
  - verify model response returns;
  - disable proxy and verify direct path still works.

Exit criteria:

- Provider proxy works end to end.
- No credentials leak in UI snapshots, logs, or error strings.
- Phase 2 call sites remain explicitly out of scope and documented.

## Phase 2 Boundary

Only after phase 1 is green should Puffer add an active `All Puffer HTTP`
control. Phase 2 must migrate and test these call sites separately:

- WebFetch;
- MCP streamable HTTP and MCP OAuth async paths;
- workflow `http_request`;
- image generation and vision workflow tools;
- Slack, Lark, Matrix, Discord, Shopify, Spotify, and similar connector tools;
- plugin marketplace and heartbeat clients if still required.

Phase 2 should move the helper into a shared crate only if real cross-crate
ownership emerges. It should add async client construction only when an actual
async call site is being migrated.
