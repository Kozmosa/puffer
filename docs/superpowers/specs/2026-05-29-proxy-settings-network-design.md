# Proxy Settings and Network Routing Design

Date: 2026-05-29
Status: design approved for implementation planning

## Intent

Add first-class proxy configuration to Puffer Desktop and the Puffer daemon so
provider traffic can reliably use a user-selected proxy. The long-term design
prioritizes stability, performance, and a single clear network abstraction over
backward compatibility with any implicit proxy behavior.

The default behavior is conservative: proxy model-related network traffic, and
only proxy all Puffer HTTP traffic when the user explicitly enables that scope.

## Goals

- Provide a native Settings UI for proxy setup that matches the current Puffer
  Settings layout.
- Support HTTP, HTTPS, SOCKS5, and SOCKS5H proxy targets.
- Recommend SOCKS5H for LLM provider traffic so DNS resolution happens through
  the proxy.
- Proxy model requests, provider model discovery, and provider auth refresh by
  default when proxy is enabled.
- Offer an explicit "All Puffer HTTP" scope for users who want WebFetch, MCP
  HTTP, workflow HTTP actions, connectors, and other outbound HTTP traffic to
  share the selected proxy.
- Keep local development and local integration endpoints off the proxy by
  default through bypass rules.
- Avoid rebuilding HTTP clients for every request by caching proxy-aware client
  builders or clients under a config hash.

## Non-Goals

- PAC file support.
- Automatic macOS or Windows system proxy import.
- Multi-proxy automatic failover.
- Per-provider proxy rules.
- Latency-based proxy ranking.
- A generic proxy rule engine.
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

2. `Scope`
   - `Model requests`
   - `All Puffer HTTP`
   - Model requests are enabled by default when proxy is enabled.
   - All Puffer HTTP is opt-in.

3. `Proxy list`
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

4. `Bypass`
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

5. `Connectivity check`
   - Tests the selected proxy against the default provider's model discovery
     endpoint when possible.
   - Falls back to the active provider base URL if discovery is unavailable.
   - Uses a short timeout, 5 to 8 seconds.

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

The saved list stores credentials in the user config unless a later credential
store abstraction is introduced. The UI must never render passwords back in
plain text after save; it should show a filled password placeholder and only
replace the stored value when the user edits that field.

## Configuration Model

Add a `network` section to `PufferConfig`.

```toml
[network.proxy]
enabled = true
scope = "model"
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

Rust types:

- `NetworkConfig`
- `ProxyConfig`
- `ProxyScope`
- `ProxyEndpoint`
- `ProxyScheme`

Validation rules:

- `enabled = true` requires `selected` to reference an existing proxy.
- `host` must be non-empty.
- `port` must be in `1..=65535`.
- `scheme` must be one of `http`, `https`, `socks5`, `socks5h`.
- Bypass entries must be valid host patterns, IPs, or CIDR ranges.
- Proxy IDs must be stable, non-empty, and unique.

Config is user-scoped. The desktop update API writes proxy settings to the user
config path, not the workspace config path.

## Network Architecture

Introduce one network construction boundary instead of adding proxy logic at
each call site.

Add a small workspace crate named `puffer-http`. Proxy-aware HTTP construction
is needed by provider, registry, OAuth, MCP, desktop API, and later connector
code, so putting it in `puffer-core` would either force awkward dependencies or
duplicate logic. The new crate should stay narrow: it owns proxy URL
construction, bypass matching, request target classification, and cached
`reqwest` client construction. It does not own provider request shaping,
session state, auth storage, or desktop UI state.

Core API:

```rust
pub enum NetworkTargetKind {
    Model,
    Discovery,
    OAuth,
    ToolHttp,
    McpHttp,
    Connector,
    InternalLocal,
}

pub struct NetworkClientFactory {
    // cached clients keyed by proxy config hash and target kind
}
```

Functions:

- build blocking client for runtime paths using `reqwest::blocking::Client`;
- build async client for MCP OAuth and async HTTP consumers using
  `reqwest::Client`;
- build proxy URL from selected endpoint;
- decide whether a request should proxy based on target kind, scope, URL, and
  bypass rules;
- redact proxy credentials for logs and snapshots.

Cargo changes:

- add `socks` feature to workspace `reqwest`;
- keep the existing TLS choice unless implementation reveals a platform trust
  requirement.

## Scope Semantics

`scope = "model"`:

- proxy `Model`;
- proxy `Discovery`;
- proxy `OAuth`;
- do not proxy `ToolHttp`, `McpHttp`, `Connector`, or `InternalLocal`.

`scope = "all"`:

- proxy external `Model`, `Discovery`, `OAuth`, `ToolHttp`, `McpHttp`, and
  `Connector` traffic;
- never proxy URLs matched by bypass;
- never proxy `InternalLocal`.

Bypass is evaluated before scope. If a URL matches bypass, it does not use the
proxy even when `scope = "all"`.

## Initial Call-Site Migration

Phase 1 must cover the high-value provider path:

- OpenAI and OpenAI-compatible streaming requests, including WorldRouter.
- Anthropic streaming requests.
- Non-streaming model HTTP request helper.
- Provider model discovery.
- Provider OAuth and auth refresh.

Phase 2 extends the same factory to other outbound HTTP:

- WebFetch.
- MCP streamable HTTP.
- workflow `http_request`.
- image generation and vision workflow tools.
- Slack, Lark, Matrix, and similar connector clients.
- plugin marketplace and heartbeat clients if still required.

Local browser Chrome DevTools probes, local MCP, Ollama, LM Studio, ComfyUI, and
other localhost services remain bypassed by default.

## Desktop API

Extend `settings-snapshot` with network proxy state:

- sanitized proxy list;
- selected proxy ID;
- enabled;
- scope;
- bypass entries;
- last known test result if available in memory.

Extend `update_config` or add a focused API:

- `save_proxy_settings`
- `test_proxy`
- `select_proxy`
- `delete_proxy`

Recommendation: add focused proxy APIs rather than overloading the generic
`update_config` object. The existing `update_config` is good for scalar values,
but proxy list edits need validation, credential redaction, and targeted error
messages.

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
- no global app startup failure for invalid proxy config.

## Performance

Do not rebuild proxy-aware HTTP clients for every request.

Daemon maintains a client cache keyed by:

- normalized proxy config hash;
- target kind;
- blocking vs async;
- timeout class when necessary.

`update_config` or proxy-specific save APIs clear the cache.

Connectivity tests use short timeouts and never block app startup.

## Testing

Required tests:

- config parse and serialize for `network.proxy`;
- validation failures for invalid scheme, port, selected ID, and bypass entry;
- snapshot redacts credentials;
- proxy URL builder supports HTTP, HTTPS, SOCKS5, and SOCKS5H;
- bypass matcher handles localhost, IPv4, IPv6, and CIDR;
- `scope = model` proxies model, discovery, and OAuth only;
- `scope = all` proxies external HTTP but skips bypassed local URLs;
- model request uses configured proxy;
- discovery request uses configured proxy;
- invalid proxy produces actionable chat error;
- Settings UI can add, edit, select, test, and save bypass entries.

## Implementation Order

1. Add config types and tests.
2. Add network target kinds, proxy URL construction, bypass matcher, and unit
   tests.
3. Add client factory and wire phase 1 provider paths.
4. Add daemon desktop API for proxy snapshot, save, select, delete, and test.
5. Add Settings `Network` pane and modal UI.
6. Add integration tests for provider request and discovery through a local test
   proxy.
7. Stop after phase 1 is green and open a follow-up spec or implementation
   ticket for phase 2 call-site expansion.

## Scope Decisions

- Proxy credentials are stored in user config for the first implementation and
  are always redacted in desktop snapshots, logs, and test-result payloads.
  Moving credentials to an OS credential store is a later hardening task because
  it would require a broader auth-storage design than this feature needs.
- Phase 1 is the implementation scope for this design: model requests,
  provider discovery, OAuth, config, daemon API, Settings UI, and tests. Phase 2
  covers the remaining external HTTP call sites and should be scheduled only
  after phase 1 is verified.
