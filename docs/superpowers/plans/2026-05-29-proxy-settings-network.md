# Proxy Settings Network Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build phase 1 proxy support for Puffer provider traffic: configure, test, persist, and route model/discovery/OAuth HTTP through a selected proxy without exposing broader "All HTTP" behavior.

**Architecture:** Add typed proxy config in `puffer-config`, a small blocking-client helper in `puffer-core::network`, and client injection into provider discovery/OAuth paths. The desktop daemon owns Settings RPCs and persistence; the Svelte Settings UI edits a local proxy draft and saves through focused daemon calls.

**Tech Stack:** Rust 2021, `reqwest` blocking client with `socks` feature, `ipnet` for CIDR bypass matching, serde/TOML config, Svelte 5, daemon WebSocket RPC, Cargo tests, Svelte typecheck.

---

## Scope And Guardrails

Implement only the phase 1 provider proxy loop from [the design spec](../specs/2026-05-29-proxy-settings-network-design.md). Do not add `scope`, `All Puffer HTTP`, async client construction, a new crate, global HTTP client caching, PAC/system proxy import, provider-specific proxy rules, or migration for WebFetch, MCP HTTP, workflow HTTP, connectors, marketplace, heartbeat, browser DevTools probes, or subscriber HTTP.

Keep every new public Rust function documented. Keep new Rust files under 1000 lines. Add component update specs in the same implementation sequence.

## File Map

- Modify `Cargo.toml`: add `socks` to workspace `reqwest`; add workspace `ipnet`.
- Modify `crates/puffer-config/Cargo.toml`: add `ipnet.workspace = true`.
- Modify `crates/puffer-config/src/lib.rs`: add proxy config structs, defaults, validation, redaction, serde tests.
- Create `crates/puffer-core/network.rs`: proxy-aware blocking client builder, bypass matcher, proxy URL builder, connectivity test.
- Modify `crates/puffer-core/lib.rs`: expose `network` only as needed by `puffer-cli`; keep helpers small.
- Modify `crates/puffer-core/Cargo.toml`: add `ipnet.workspace = true` and `urlencoding = "2.1.3"`.
- Modify `crates/puffer-provider-registry/src/discovery.rs`: add `ModelDiscoveryClient::with_client`.
- Modify `crates/puffer-provider-registry/src/registry.rs`: add `_with_client` discovery entrypoints used by daemon/CLI.
- Modify `crates/puffer-provider-openai/src/auth.rs` and `src/lib.rs`: add OAuth exchange/refresh helpers that accept an injected blocking client.
- Modify `crates/puffer-transport-anthropic/src/auth.rs` and `src/lib.rs`: add OAuth exchange/refresh/profile/role helpers that accept an injected blocking client.
- Modify `crates/puffer-core/runtime/http_support.rs`: use proxy-aware clients for non-streaming model sends.
- Modify `crates/puffer-core/runtime/openai.rs`: use proxy-aware clients for streaming OpenAI sends.
- Modify `crates/puffer-core/runtime/anthropic.rs`: use proxy-aware clients for streaming Anthropic sends.
- Modify `crates/puffer-core/runtime/claude_tools/web_search.rs`: route provider-backed web-search sends through the same helper.
- Modify `crates/puffer-cli/src/daemon.rs`: build proxy-aware discovery/OAuth clients; add `save_proxy_settings` and `test_proxy` RPCs.
- Modify `crates/puffer-cli/src/desktop_api.rs` and `desktop_api_types.rs`: include sanitized proxy state in Settings snapshots.
- Modify `crates/puffer-cli/src/main.rs`: use proxy-aware discovery/OAuth refresh where CLI config is available.
- Modify `apps/puffer-desktop/src/lib/types.ts`: add proxy state and RPC payload types.
- Modify `apps/puffer-desktop/src/lib/api/desktop.ts`: add `saveProxySettings` and `testProxy` client calls.
- Create `apps/puffer-desktop/src/lib/screens/settings/NetworkSettings.svelte`: isolated network pane UI.
- Modify `apps/puffer-desktop/src/lib/screens/Settings.svelte`: add `Network` nav item and render the new component.
- Modify `apps/puffer-desktop/src/lib/mockData.ts`: add sanitized proxy mock data.
- Create/update component specs:
  - `specs/puffer-config/04.md`
  - `specs/puffer-core/211.md`
  - `specs/puffer-provider-registry/06.md`
  - `specs/puffer-provider-openai/04.md`
  - `specs/puffer-transport-anthropic/01.md`
  - `specs/puffer-cli/119.md`
  - `specs/puffer-desktop/586.md`

## Task 1: Config Contract

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/puffer-config/Cargo.toml`
- Modify: `crates/puffer-config/src/lib.rs`
- Create: `specs/puffer-config/04.md`

- [ ] **Step 1: Add dependencies**

Add `socks` and `ipnet` to the workspace dependency block:

```toml
ipnet = "2.11.0"
reqwest = { version = "0.12.24", default-features = false, features = ["blocking", "json", "multipart", "rustls-tls", "socks"] }
```

Add `ipnet` to `crates/puffer-config/Cargo.toml`:

```toml
ipnet.workspace = true
```

- [ ] **Step 2: Write failing config tests**

Append tests in `crates/puffer-config/src/lib.rs` under `mod tests`:

```rust
    #[test]
    fn proxy_config_defaults_to_disabled_with_bypass_entries() {
        let config = PufferConfig::default();
        assert!(!config.network.proxy.enabled);
        assert_eq!(config.network.proxy.selected, None);
        assert!(config
            .network
            .proxy
            .bypass
            .iter()
            .any(|entry| entry == "127.0.0.1"));
        assert!(config.network.proxy.proxies.is_empty());
    }

    #[test]
    fn proxy_config_round_trips_through_toml() {
        let raw = r#"
app_name = "Puffer Code"
default_provider = "anthropic"
theme = "puffer"

[memory]
enabled = true
char_limit = 6000
review_nudge_interval = 8
flush_min_turns = 6
background_review = true
flush_on_compact = true

[recap]
enabled = true
auto = true
idle_secs = 180
min_user_messages = 3
cooldown_messages = 2

[browser]

[mascot]
id = "clawd"
display_name = "Clawd"
enabled = true

[ui]
no_alt_screen = false
tmux_golden_mode = false

[network.proxy]
enabled = true
selected = "local-socks"
bypass = ["localhost", "127.0.0.1", "::1", "10.0.0.0/8"]

[[network.proxy.proxies]]
id = "local-socks"
scheme = "socks5h"
host = "127.0.0.1"
port = 7890
username = "alice"
password = "secret"
"#;
        let config: PufferConfig = toml::from_str(raw).expect("config");
        assert!(config.network.proxy.enabled);
        assert_eq!(config.network.proxy.selected.as_deref(), Some("local-socks"));
        assert_eq!(config.network.proxy.proxies[0].scheme, ProxyScheme::Socks5h);
        assert_eq!(config.network.proxy.proxies[0].host, "127.0.0.1");
        assert_eq!(config.network.proxy.proxies[0].port, 7890);
        assert_eq!(config.network.proxy.proxies[0].username.as_deref(), Some("alice"));
        assert_eq!(config.network.proxy.proxies[0].password.as_deref(), Some("secret"));

        let serialized = toml::to_string(&config).expect("serialize");
        assert!(serialized.contains("[network.proxy]"));
        assert!(serialized.contains("[[network.proxy.proxies]]"));
    }

    #[test]
    fn proxy_config_validation_rejects_bad_values() {
        let mut config = PufferConfig::default();
        config.network.proxy.enabled = true;
        assert!(config.network.proxy.validate().is_err());

        config.network.proxy.selected = Some("missing".to_string());
        config.network.proxy.proxies.push(ProxyEndpoint {
            id: "local-socks".to_string(),
            scheme: ProxyScheme::Socks5h,
            host: "127.0.0.1".to_string(),
            port: 7890,
            username: None,
            password: None,
        });
        assert!(config.network.proxy.validate().is_err());

        config.network.proxy.selected = Some("local-socks".to_string());
        config.network.proxy.proxies.push(ProxyEndpoint {
            id: "local-socks".to_string(),
            scheme: ProxyScheme::Http,
            host: "example.com".to_string(),
            port: 8080,
            username: None,
            password: None,
        });
        assert!(config.network.proxy.validate().is_err());
    }

    #[test]
    fn sanitized_proxy_snapshot_redacts_credentials() {
        let endpoint = ProxyEndpoint {
            id: "office".to_string(),
            scheme: ProxyScheme::Http,
            host: "proxy.example".to_string(),
            port: 8080,
            username: Some("alice".to_string()),
            password: Some("secret".to_string()),
        };
        let sanitized = endpoint.sanitized();
        assert_eq!(sanitized.username.as_deref(), Some("alice"));
        assert!(sanitized.has_password);
        assert_eq!(sanitized.uri, "http://proxy.example:8080");
    }
```

- [ ] **Step 3: Run failing tests**

Run:

```bash
cargo test -p puffer-config proxy_config -- --nocapture
```

Expected: fail because `network`, `ProxyScheme`, `ProxyEndpoint`, validation, and sanitization do not exist.

- [ ] **Step 4: Implement config types**

Add these types near the existing config structs in `crates/puffer-config/src/lib.rs` and add `pub network: NetworkConfig` to `PufferConfig`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NetworkConfig {
    #[serde(default)]
    pub proxy: ProxyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub selected: Option<String>,
    #[serde(default = "default_proxy_bypass")]
    pub bypass: Vec<String>,
    #[serde(default)]
    pub proxies: Vec<ProxyEndpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyEndpoint {
    pub id: String,
    pub scheme: ProxyScheme,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProxyScheme {
    Http,
    Https,
    Socks5,
    Socks5h,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SanitizedProxyEndpoint {
    pub id: String,
    pub scheme: ProxyScheme,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub has_password: bool,
    pub uri: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            selected: None,
            bypass: default_proxy_bypass(),
            proxies: Vec::new(),
        }
    }
}

impl ProxyScheme {
    /// Returns the URI scheme string accepted by reqwest proxy configuration.
    pub fn as_uri_scheme(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Socks5 => "socks5",
            Self::Socks5h => "socks5h",
        }
    }
}

impl ProxyEndpoint {
    /// Returns a credential-free endpoint snapshot suitable for UI payloads.
    pub fn sanitized(&self) -> SanitizedProxyEndpoint {
        SanitizedProxyEndpoint {
            id: self.id.clone(),
            scheme: self.scheme,
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone().filter(|value| !value.trim().is_empty()),
            has_password: self
                .password
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            uri: format!("{}://{}:{}", self.scheme.as_uri_scheme(), self.host, self.port),
        }
    }
}

impl ProxyConfig {
    /// Validates selected proxy, endpoint identity, endpoint address, and bypass entries.
    pub fn validate(&self) -> Result<()> {
        let mut ids = std::collections::BTreeSet::new();
        for endpoint in &self.proxies {
            if endpoint.id.trim().is_empty() {
                anyhow::bail!("proxy id must not be empty");
            }
            if !ids.insert(endpoint.id.as_str()) {
                anyhow::bail!("duplicate proxy id `{}`", endpoint.id);
            }
            if endpoint.host.trim().is_empty() {
                anyhow::bail!("proxy `{}` host must not be empty", endpoint.id);
            }
            if endpoint.port == 0 {
                anyhow::bail!("proxy `{}` port must be between 1 and 65535", endpoint.id);
            }
        }
        if self.enabled {
            let selected = self
                .selected
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("enabled proxy requires selected proxy id"))?;
            if !ids.contains(selected) {
                anyhow::bail!("selected proxy `{selected}` does not exist");
            }
        }
        for entry in &self.bypass {
            validate_bypass_entry(entry)?;
        }
        Ok(())
    }
}

fn default_proxy_bypass() -> Vec<String> {
    vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
        "10.0.0.0/8".to_string(),
        "172.16.0.0/12".to_string(),
        "192.168.0.0/16".to_string(),
    ]
}

fn validate_bypass_entry(entry: &str) -> Result<()> {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if trimmed.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }
    if trimmed.parse::<ipnet::IpNet>().is_ok() {
        return Ok(());
    }
    if trimmed.contains('*') || trimmed.contains('/') || trimmed.contains('\\') {
        anyhow::bail!("invalid proxy bypass entry `{trimmed}`");
    }
    Ok(())
}
```

Set `network: NetworkConfig::default()` in `PufferConfig::default()`. Update `load_config` user-selection preservation to include `config.network.clone()` so user proxy settings win over workspace config.

- [ ] **Step 5: Run config tests**

Run:

```bash
cargo test -p puffer-config proxy_config -- --nocapture
cargo test -p puffer-config load_config_preserves_user_provider_selection_over_workspace_defaults -- --nocapture
```

Expected: pass.

- [ ] **Step 6: Add component update spec**

Create `specs/puffer-config/04.md`:

```markdown
# 04 - Network Proxy Config

Adds the user-scoped `network.proxy` config section for provider proxy settings.

- `ProxyConfig` stores `enabled`, `selected`, default bypass entries, and a list of proxy endpoints.
- `ProxyEndpoint` supports `http`, `https`, `socks5`, and `socks5h`.
- Validation rejects missing selected proxies, duplicate IDs, empty hosts, port zero, and malformed bypass entries.
- Sanitized endpoint snapshots expose URI and password presence without exposing password values.
- User-level proxy settings are preserved when workspace config is layered over user config.
```

- [ ] **Step 7: Commit config contract**

```bash
git add Cargo.toml crates/puffer-config/Cargo.toml crates/puffer-config/src/lib.rs specs/puffer-config/04.md
git commit -m "feat(config): add provider proxy settings"
```

## Task 2: Core Network Helper

**Files:**
- Create: `crates/puffer-core/network.rs`
- Modify: `crates/puffer-core/lib.rs`
- Modify: `crates/puffer-core/Cargo.toml`
- Create: `specs/puffer-core/211.md`

- [ ] **Step 1: Write failing network tests**

Create `crates/puffer-core/network.rs` with tests first:

```rust
use anyhow::{Context, Result};
use puffer_config::{ProxyConfig, ProxyEndpoint, ProxyScheme};
use reqwest::blocking::Client;
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// Describes why an HTTP client is being constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpPurpose {
    Model,
    Discovery,
    OAuth,
    ConnectivityTest,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy_config() -> ProxyConfig {
        ProxyConfig {
            enabled: true,
            selected: Some("local".to_string()),
            bypass: vec!["localhost".to_string(), "10.0.0.0/8".to_string()],
            proxies: vec![ProxyEndpoint {
                id: "local".to_string(),
                scheme: ProxyScheme::Socks5h,
                host: "127.0.0.1".to_string(),
                port: 7890,
                username: Some("user".to_string()),
                password: Some("pass".to_string()),
            }],
        }
    }

    #[test]
    fn proxy_uri_includes_encoded_credentials() {
        let endpoint = ProxyEndpoint {
            id: "auth".to_string(),
            scheme: ProxyScheme::Http,
            host: "proxy.example".to_string(),
            port: 8080,
            username: Some("user name".to_string()),
            password: Some("p@ss".to_string()),
        };
        assert_eq!(
            proxy_uri(&endpoint).expect("uri"),
            "http://user%20name:p%40ss@proxy.example:8080"
        );
    }

    #[test]
    fn bypass_matches_localhost_and_cidr() {
        let config = proxy_config();
        assert!(bypass_matches(&config, "http://localhost:3000/health"));
        assert!(bypass_matches(&config, "http://10.2.3.4/v1/models"));
        assert!(!bypass_matches(&config, "https://api.openai.com/v1/responses"));
    }

    #[test]
    fn client_builder_accepts_selected_proxy() {
        let config = proxy_config();
        let client = blocking_client(&config, HttpPurpose::Model, Duration::from_secs(30))
            .expect("client");
        let _ = client;
    }
}
```

- [ ] **Step 2: Run failing network tests**

Run:

```bash
cargo test -p puffer-core network::tests -- --nocapture
```

Expected: fail because the module is not wired into `lib.rs` and helper functions are missing.

- [ ] **Step 3: Implement network helper**

Complete `crates/puffer-core/network.rs`:

```rust
use anyhow::{Context, Result};
use puffer_config::{ProxyConfig, ProxyEndpoint, ProxyScheme};
use reqwest::blocking::Client;
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// Describes why an HTTP client is being constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpPurpose {
    Model,
    Discovery,
    OAuth,
    ConnectivityTest,
}

/// Builds a blocking reqwest client using the selected proxy when enabled and not bypassed.
pub fn blocking_client(
    proxy: &ProxyConfig,
    purpose: HttpPurpose,
    timeout: Duration,
) -> Result<Client> {
    let mut builder = Client::builder().timeout(timeout);
    if proxy.enabled {
        if let Some(endpoint) = selected_endpoint(proxy)? {
            builder = builder.proxy(reqwest::Proxy::all(proxy_uri(endpoint)?)?);
        }
    }
    let _ = purpose;
    builder.build().context("failed to build HTTP client")
}

/// Builds a blocking reqwest client for one target URL, honoring bypass entries.
pub fn blocking_client_for_url(
    proxy: &ProxyConfig,
    purpose: HttpPurpose,
    url: &str,
    timeout: Duration,
) -> Result<Client> {
    if proxy.enabled && !bypass_matches(proxy, url) {
        blocking_client(proxy, purpose, timeout)
    } else {
        Client::builder()
            .timeout(timeout)
            .build()
            .context("failed to build HTTP client")
    }
}

/// Tests a proxy endpoint against a URL and returns elapsed milliseconds.
pub fn test_proxy_endpoint(
    endpoint: &ProxyEndpoint,
    target_url: &str,
    timeout: Duration,
) -> Result<u128> {
    let mut config = ProxyConfig::default();
    config.enabled = true;
    config.selected = Some(endpoint.id.clone());
    config.proxies = vec![endpoint.clone()];
    let started = Instant::now();
    let client = blocking_client_for_url(&config, HttpPurpose::ConnectivityTest, target_url, timeout)?;
    let response = client
        .get(target_url)
        .send()
        .with_context(|| format!("proxy test request to {target_url} failed"))?;
    if !response.status().is_success() {
        anyhow::bail!("proxy test failed with HTTP {}", response.status());
    }
    Ok(started.elapsed().as_millis())
}

/// Returns true when the URL host matches a configured bypass entry.
pub fn bypass_matches(proxy: &ProxyConfig, url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    proxy.bypass.iter().any(|entry| bypass_entry_matches(entry, host))
}

/// Builds the proxy URI accepted by reqwest.
pub fn proxy_uri(endpoint: &ProxyEndpoint) -> Result<String> {
    if endpoint.host.trim().is_empty() {
        anyhow::bail!("proxy host must not be empty");
    }
    let scheme = endpoint.scheme.as_uri_scheme();
    let host = endpoint.host.trim();
    let auth = match (
        endpoint.username.as_deref().filter(|value| !value.is_empty()),
        endpoint.password.as_deref().filter(|value| !value.is_empty()),
    ) {
        (Some(username), Some(password)) => format!(
            "{}:{}@",
            urlencoding::encode(username),
            urlencoding::encode(password)
        ),
        (Some(username), None) => format!("{}@", urlencoding::encode(username)),
        _ => String::new(),
    };
    Ok(format!("{scheme}://{auth}{host}:{}", endpoint.port))
}

fn selected_endpoint(proxy: &ProxyConfig) -> Result<Option<&ProxyEndpoint>> {
    let Some(selected) = proxy.selected.as_deref() else {
        return Ok(None);
    };
    proxy
        .proxies
        .iter()
        .find(|endpoint| endpoint.id == selected)
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("selected proxy `{selected}` does not exist"))
}

fn bypass_entry_matches(entry: &str, host: &str) -> bool {
    let entry = entry.trim();
    if entry.is_empty() {
        return false;
    }
    if entry.eq_ignore_ascii_case(host) {
        return true;
    }
    let Ok(host_ip) = host.parse::<IpAddr>() else {
        return false;
    };
    if let Ok(entry_ip) = entry.parse::<IpAddr>() {
        return host_ip == entry_ip;
    }
    if let Ok(net) = entry.parse::<ipnet::IpNet>() {
        return net.contains(&host_ip);
    }
    false
}
```

Add `mod network;` and `pub use network::{blocking_client_for_url, test_proxy_endpoint, HttpPurpose};` to `crates/puffer-core/lib.rs`. Add dependencies in `crates/puffer-core/Cargo.toml`:

```toml
ipnet.workspace = true
urlencoding = "2.1.3"
```

- [ ] **Step 4: Run network tests**

Run:

```bash
cargo test -p puffer-core network::tests -- --nocapture
```

Expected: pass.

- [ ] **Step 5: Add component update spec**

Create `specs/puffer-core/211.md`:

```markdown
# 211 - Provider Proxy HTTP Helper

Adds `puffer-core::network` as the phase 1 provider proxy boundary.

- Builds blocking `reqwest` clients with the selected configured proxy.
- Supports bypass matching for exact hosts, IPs, and CIDR ranges.
- Provides credential-safe proxy URI construction and Settings connectivity tests.
- Does not add async clients, global caching, tool HTTP migration, MCP migration, or connector migration.
```

- [ ] **Step 6: Commit core helper**

```bash
git add Cargo.toml crates/puffer-core/Cargo.toml crates/puffer-core/lib.rs crates/puffer-core/network.rs specs/puffer-core/211.md
git commit -m "feat(core): add provider proxy HTTP helper"
```

## Task 3: Discovery Client Injection

**Files:**
- Modify: `crates/puffer-provider-registry/src/discovery.rs`
- Modify: `crates/puffer-provider-registry/src/registry.rs`
- Create: `specs/puffer-provider-registry/06.md`

- [ ] **Step 1: Write failing discovery injection test**

Add this test in `crates/puffer-provider-registry/src/discovery.rs` tests:

```rust
    #[test]
    fn discovery_client_accepts_injected_reqwest_client() {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(1))
            .build()
            .expect("client");
        let discovery = ModelDiscoveryClient::with_client(client);
        let _ = discovery;
    }
```

- [ ] **Step 2: Run failing registry tests**

```bash
cargo test -p puffer-provider-registry discovery_client_accepts_injected_reqwest_client -- --nocapture
```

Expected: fail because `with_client` does not exist.

- [ ] **Step 3: Add client injection**

In `ModelDiscoveryClient` impl, add:

```rust
    /// Creates a discovery client from an externally configured blocking client.
    pub fn with_client(client: Client) -> Self {
        Self { client }
    }
```

In `registry.rs`, add provider-specific methods that mirror existing methods but accept a `&ModelDiscoveryClient`:

```rust
    /// Discovers and merges runtime models using an externally configured discovery client.
    pub fn discover_and_merge_provider_with_discovery_client(
        &mut self,
        provider_id: &str,
        auth_store: &AuthStore,
        client: &ModelDiscoveryClient,
    ) -> Result<()> {
        self.discover_and_merge_provider_with_client(provider_id, auth_store, client)
    }
```

In `registry.rs`, extract the result merge/cache portion of `discover_and_merge_all` into a private helper so injected-client discovery keeps the current cache behavior:

```rust
    fn apply_discovery_results(
        &mut self,
        results: Vec<(String, Result<Vec<ModelDescriptor>>)>,
    ) -> Result<()> {
        let mut failures = Vec::new();
        let mut cache_updates: HashMap<String, DiscoveryCacheEntry> = HashMap::new();
        let now_ms = current_time_ms();

        for (provider_id, result) in results {
            match result {
                Ok(discovered) => {
                    cache_updates.insert(
                        provider_id.clone(),
                        DiscoveryCacheEntry {
                            models: discovered.clone(),
                            cached_at_ms: now_ms,
                        },
                    );
                    if !discovered.is_empty() {
                        if let Some(entry) = self.providers.get_mut(&provider_id) {
                            merge_discovered_models(&mut entry.descriptor.models, discovered);
                        }
                    }
                }
                Err(error) => failures.push(format!("{provider_id}: {error}")),
            }
        }

        if !cache_updates.is_empty() {
            persist_cache_updates(&cache_updates);
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(anyhow!(
                "model discovery failed for {} provider(s): {}",
                failures.len(),
                failures.join("; ")
            ))
        }
    }
```

Replace the duplicated merge/cache block in `discover_and_merge_all` with:

```rust
let results = run_parallel_discovery(&eligible, auth_store);
self.apply_discovery_results(results)
```

For all-provider discovery, add a serial variant for proxy-aware callers that uses the same merge/cache helper:

```rust
    /// Discovers stale providers with one externally configured client.
    pub fn discover_and_merge_all_with_discovery_client(
        &mut self,
        auth_store: &AuthStore,
        client: &ModelDiscoveryClient,
    ) -> Result<()> {
        let stale_ids = self.apply_discovery_cache();
        let eligible = self.discovery_eligible(&stale_ids, auth_store);
        if eligible.is_empty() {
            return Ok(());
        }
        let results = eligible
            .iter()
            .map(|provider| {
                (
                    provider.id.clone(),
                    client.discover_models(provider, auth_store),
                )
            })
            .collect();
        self.apply_discovery_results(results)
    }
```

Add proxy-aware background discovery next to `start_background_discovery`:

```rust
    /// Spawns background discovery using an externally configured discovery client.
    pub fn start_background_discovery_with_discovery_client(
        &self,
        provider_ids: Vec<String>,
        auth_store: &AuthStore,
        client: ModelDiscoveryClient,
    ) -> Option<std::thread::JoinHandle<()>> {
        let eligible = self.discovery_eligible(&provider_ids, auth_store);
        if eligible.is_empty() {
            return None;
        }
        let auth = auth_store.clone();
        Some(std::thread::spawn(move || {
            let now_ms = current_time_ms();
            let mut cache_updates: HashMap<String, DiscoveryCacheEntry> = HashMap::new();
            for provider in eligible {
                if let Ok(models) = client.discover_models(&provider, &auth) {
                    cache_updates.insert(
                        provider.id.clone(),
                        DiscoveryCacheEntry {
                            models,
                            cached_at_ms: now_ms,
                        },
                    );
                }
            }
            if !cache_updates.is_empty() {
                persist_cache_updates(&cache_updates);
            }
        }))
    }
```

- [ ] **Step 4: Run provider registry tests**

```bash
cargo test -p puffer-provider-registry discovery_client_accepts_injected_reqwest_client -- --nocapture
cargo test -p puffer-provider-registry discover_and_merge -- --nocapture
```

Expected: pass.

- [ ] **Step 5: Add component update spec**

Create `specs/puffer-provider-registry/06.md`:

```markdown
# 06 - Proxy-Aware Discovery Injection

Adds externally configured discovery-client injection for provider model discovery.

- `ModelDiscoveryClient::with_client` accepts proxy-aware blocking clients from daemon/CLI callers.
- Provider-specific discovery can use injected clients without making the registry depend on `puffer-core`.
- Default discovery behavior remains available through `ModelDiscoveryClient::new`.
```

- [ ] **Step 6: Commit discovery injection**

```bash
git add crates/puffer-provider-registry/src/discovery.rs crates/puffer-provider-registry/src/registry.rs specs/puffer-provider-registry/06.md
git commit -m "feat(provider-registry): allow proxy-aware discovery clients"
```

## Task 4: OAuth Client Injection

**Files:**
- Modify: `crates/puffer-provider-openai/src/auth.rs`
- Modify: `crates/puffer-provider-openai/src/lib.rs`
- Create: `specs/puffer-provider-openai/04.md`
- Modify: `crates/puffer-transport-anthropic/src/auth.rs`
- Modify: `crates/puffer-transport-anthropic/src/lib.rs`
- Create: `specs/puffer-transport-anthropic/01.md`

- [ ] **Step 1: Write failing OpenAI OAuth injection tests**

In `crates/puffer-provider-openai/src/auth.rs` tests, add:

```rust
    #[test]
    fn refresh_oauth_token_with_client_builds_request_path() {
        let client = codex_oauth_client().expect("client");
        let result = refresh_oauth_token_with_client(&client, "refresh-token");
        assert!(result.is_err());
    }
```

This intentionally errors without a test server but proves the helper exists and accepts a caller-provided client.

- [ ] **Step 2: Write failing Anthropic OAuth injection tests**

In `crates/puffer-transport-anthropic/src/auth.rs` tests, add:

```rust
    #[test]
    fn refresh_oauth_token_with_client_accepts_injected_client() {
        let client = Client::builder().build().expect("client");
        let result = refresh_oauth_token_with_client(&client, "refresh-token", None, None);
        assert!(result.is_err());
    }
```

- [ ] **Step 3: Run failing OAuth tests**

```bash
cargo test -p puffer-provider-openai refresh_oauth_token_with_client_builds_request_path -- --nocapture
cargo test -p puffer-transport-anthropic refresh_oauth_token_with_client_accepts_injected_client -- --nocapture
```

Expected: fail because `_with_client` helpers do not exist.

- [ ] **Step 4: Implement OpenAI OAuth injection**

In `crates/puffer-provider-openai/src/auth.rs`, keep existing public functions and route them through new internal helpers:

```rust
pub(crate) fn exchange_authorization_code_with_client(
    client: &Client,
    code: &str,
    verifier: &str,
    redirect_uri: Option<&str>,
) -> Result<OpenAIOAuthCredentials> {
    let response = client
        .post(OPENAI_TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri.unwrap_or(OPENAI_REDIRECT_URI)),
            ("client_id", OPENAI_CODEX_CLIENT_ID),
        ])
        .send()
        .context("failed to exchange OpenAI authorization code")?;
    parse_token_response(response, "OpenAI token exchange")
}

pub(crate) fn refresh_oauth_token_with_client(
    client: &Client,
    refresh_token: &str,
) -> Result<OpenAIOAuthCredentials> {
    let response = client
        .post(openai_refresh_token_url())
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", OPENAI_CODEX_CLIENT_ID),
        ])
        .send()
        .context("failed to refresh OpenAI OAuth token")?;
    parse_token_response(response, "OpenAI token refresh")
}
```

Extract the current duplicated response parsing into:

```rust
fn parse_token_response(response: reqwest::blocking::Response, label: &str) -> Result<OpenAIOAuthCredentials> {
    let status = response.status();
    let payload: OpenAITokenResponse = response
        .json()
        .with_context(|| format!("failed to parse {label} response"))?;
    if !status.is_success() {
        return Err(anyhow!("{label} failed with status {status}"));
    }
    token_response_to_credentials(payload)
}
```

In `src/lib.rs`, export public wrappers:

```rust
/// Exchanges an OAuth authorization code using an injected blocking HTTP client.
pub fn exchange_authorization_code_with_client(
    client: &reqwest::blocking::Client,
    code: &str,
    verifier: &str,
    redirect_uri: Option<&str>,
) -> anyhow::Result<OpenAIOAuthCredentials> {
    auth::exchange_authorization_code_with_client(client, code, verifier, redirect_uri)
}

/// Refreshes OpenAI bearer credentials using an injected blocking HTTP client.
pub fn refresh_oauth_token_with_client(
    client: &reqwest::blocking::Client,
    refresh_token: &str,
) -> anyhow::Result<OpenAIOAuthCredentials> {
    auth::refresh_oauth_token_with_client(client, refresh_token)
}
```

- [ ] **Step 5: Implement Anthropic OAuth injection**

In `crates/puffer-transport-anthropic/src/auth.rs`, add `_with_client` variants for token exchange and refresh. Keep existing public behavior by building `Client::new()` and forwarding.

Required signatures:

```rust
pub fn exchange_authorization_code_with_client(
    client: &Client,
    code: &str,
    verifier: &str,
    state: &str,
    redirect_uri: Option<&str>,
    base_api_url: Option<&str>,
) -> Result<AnthropicOAuthCredentials>

pub fn refresh_oauth_token_with_client(
    client: &Client,
    refresh_token: &str,
    scopes: Option<&[String]>,
    existing: Option<&AnthropicOAuthCredentials>,
) -> Result<AnthropicOAuthCredentials>
```

When enrichment needs profile, roles, or API-key creation, pass the same injected client through private helper variants:

```rust
fn fetch_oauth_profile_with_client(client: &Client, base_api_url: &str, access_token: &str) -> Result<AnthropicOAuthProfile>
fn fetch_user_roles_with_client(client: &Client, base_api_url: &str, access_token: &str) -> Result<AnthropicUserRoles>
fn create_api_key_with_client(client: &Client, base_api_url: &str, access_token: &str, organization_id: &str) -> Result<String>
```

In `src/lib.rs`, export the two public `_with_client` functions.

- [ ] **Step 6: Run OAuth tests**

```bash
cargo test -p puffer-provider-openai refresh_oauth_token -- --nocapture
cargo test -p puffer-transport-anthropic oauth -- --nocapture
```

Expected: pass existing and new OAuth tests.

- [ ] **Step 7: Add component update specs**

Create `specs/puffer-provider-openai/04.md`:

```markdown
# 04 - Proxy-Aware OAuth Client Injection

Adds OpenAI OAuth exchange and refresh helpers that accept an injected blocking reqwest client.

- Existing public OAuth helpers keep their default behavior.
- Daemon/CLI callers can pass proxy-aware clients without making this crate depend on puffer-core.
- Token response parsing remains shared between default and injected-client paths.
```

Create `specs/puffer-transport-anthropic/01.md`:

```markdown
# 01 - Proxy-Aware Anthropic OAuth Client Injection

Adds Anthropic OAuth exchange and refresh helpers that accept an injected blocking reqwest client.

- Existing public helpers keep their default behavior.
- Profile, role, and API-key enrichment use the injected client when called from proxy-aware flows.
- Anthropic request construction and compatibility behavior remain unchanged.
```

- [ ] **Step 8: Commit OAuth injection**

```bash
git add crates/puffer-provider-openai/src/auth.rs crates/puffer-provider-openai/src/lib.rs specs/puffer-provider-openai/04.md crates/puffer-transport-anthropic/src/auth.rs crates/puffer-transport-anthropic/src/lib.rs specs/puffer-transport-anthropic/01.md
git commit -m "feat(auth): allow proxy-aware OAuth clients"
```

## Task 5: Model Runtime Wiring

**Files:**
- Modify: `crates/puffer-core/runtime/http_support.rs`
- Modify: `crates/puffer-core/runtime/openai.rs`
- Modify: `crates/puffer-core/runtime/anthropic.rs`
- Modify: `crates/puffer-core/runtime/claude_tools/web_search.rs`

- [ ] **Step 1: Add tests for proxy-aware send helpers**

Add unit tests near existing HTTP retry tests in `crates/puffer-core/runtime/tests/http_retries.rs` or a new `crates/puffer-core/runtime/tests/proxy_http.rs` module:

```rust
#[test]
fn raw_http_request_uses_configured_proxy_for_provider_url() {
    let proxy = puffer_config::ProxyConfig {
        enabled: true,
        selected: Some("local".to_string()),
        bypass: vec![],
        proxies: vec![puffer_config::ProxyEndpoint {
            id: "local".to_string(),
            scheme: puffer_config::ProxyScheme::Http,
            host: "127.0.0.1".to_string(),
            port: 9,
            username: None,
            password: None,
        }],
    };
    let result = super::super::send_http_request_raw_with_proxy(
        "https://api.openai.com/v1/responses",
        &[],
        "{}",
        false,
        &proxy,
    );
    assert!(result.is_err());
}
```

This test should compile only after adding `send_http_request_raw_with_proxy`.

- [ ] **Step 2: Run failing core runtime test**

```bash
cargo test -p puffer-core raw_http_request_uses_configured_proxy_for_provider_url -- --nocapture
```

Expected: fail because proxy-aware helper does not exist.

- [ ] **Step 3: Add proxy-aware raw HTTP helpers**

In `runtime/http_support.rs`, add overload-style functions:

```rust
pub(crate) fn send_http_request_raw_with_proxy(
    url: &str,
    headers: &[(String, String)],
    body: &str,
    anthropic: bool,
    proxy: &puffer_config::ProxyConfig,
) -> Result<RawHttpResponse> {
    trace_http_exchange("request", url, headers, body);
    let retry_config = http_retry_config();
    let total_attempts = retry_config.retries.saturating_add(1);
    for attempt in 1..=total_attempts {
        match send_http_request_raw_once_with_proxy(url, headers, body, anthropic, proxy) {
            Ok(response) => {
                trace_http_response(url, response.status.as_u16(), &response.text);
                let status = response.status.as_u16();
                let provider = if anthropic { "anthropic" } else { "openai" };
                if quota::classify_response(provider, status, &response.text).is_some() {
                    return Ok(response);
                }
                if attempt < total_attempts && (status == 429 || (500..=599).contains(&status)) {
                    let delay = retry_delay(retry_config, attempt);
                    if !delay.is_zero() {
                        std::thread::sleep(delay);
                    }
                    continue;
                }
                return Ok(response);
            }
            Err(error) if attempt < total_attempts && is_retryable_http_error(&error) => {
                trace_http_retry(url, attempt, &error);
                let delay = retry_delay(retry_config, attempt);
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("http retry loop exited without returning")
}
```

Implement `send_http_request_raw_once_with_proxy` by replacing `Client::builder()` with:

```rust
let client = crate::network::blocking_client_for_url(
    proxy,
    crate::network::HttpPurpose::Model,
    url,
    Duration::from_secs(300),
)
.unwrap_or_else(|_| Client::new());
```

Keep existing `send_http_request_raw` as a wrapper using `ProxyConfig::default()`.

- [ ] **Step 4: Wire OpenAI streaming sends**

In `runtime/openai.rs`, update `send_openai_request_stream_raw` to accept `proxy: &puffer_config::ProxyConfig` and build the client with `blocking_client_for_url(proxy, HttpPurpose::Model, url, openai_stream_read_timeout())`. Pass `&state.config.network.proxy` from the calling paths.

For retry closures that rebuild requests, keep the same `client` value captured outside the closure.

- [ ] **Step 5: Wire Anthropic streaming sends**

In `runtime/anthropic.rs`, replace direct `Client::builder()` in `one_turn_streaming` with:

```rust
let client = crate::network::blocking_client_for_url(
    &state.config.network.proxy,
    crate::network::HttpPurpose::Model,
    &self.request_url,
    std::time::Duration::from_secs(300),
)
.unwrap_or_else(|_| Client::new());
```

Keep request headers and Anthropic-specific header ordering logic unchanged.

- [ ] **Step 6: Wire provider-backed web search**

In `runtime/claude_tools/web_search.rs`, change the public tool executors to accept the active proxy config:

```rust
pub fn execute_claude_openai_web_search(
    request_config: &OpenAIRequestConfig,
    model_id: &str,
    raw_input: Value,
    proxy: &puffer_config::ProxyConfig,
) -> Result<String>

pub fn execute_claude_anthropic_web_search(
    request_config: &AnthropicRequestConfig,
    model_id: &str,
    raw_input: Value,
    proxy: &puffer_config::ProxyConfig,
) -> Result<String>
```

Replace `Client::new()` in `send_json_request` with a parameterized client:

```rust
fn send_json_request_with_client(
    client: &Client,
    url: &str,
    headers: &[(String, String)],
    body: &str,
    anthropic: bool,
) -> Result<Value>
```

In both executor functions, build the client before sending:

```rust
let client = crate::network::blocking_client_for_url(
    proxy,
    crate::network::HttpPurpose::Model,
    &request.url,
    std::time::Duration::from_secs(300),
)
.unwrap_or_else(|_| Client::new());
```

Call `send_json_request_with_client(&client, ...)`.

In `runtime/claude_tools/mod.rs`, update both `WebSearch` match blocks so OpenAI and Anthropic provider-tool calls pass `&state.config.network.proxy` into the web-search executor. Do not change `WebFetch`, workflow HTTP tools, or MCP tools in this phase.

- [ ] **Step 7: Run runtime tests**

```bash
cargo test -p puffer-core raw_http_request_uses_configured_proxy_for_provider_url -- --nocapture
cargo test -p puffer-core http_retries -- --nocapture
cargo test -p puffer-transport-anthropic --test claude_parity -- --nocapture
```

Expected: pass.

- [ ] **Step 8: Commit runtime wiring**

```bash
git add crates/puffer-core/runtime/http_support.rs crates/puffer-core/runtime/openai.rs crates/puffer-core/runtime/anthropic.rs crates/puffer-core/runtime/claude_tools/web_search.rs
git commit -m "feat(core): route provider model HTTP through proxy config"
```

## Task 6: Daemon And CLI Discovery/OAuth Wiring

**Files:**
- Modify: `crates/puffer-cli/src/daemon.rs`
- Modify: `crates/puffer-cli/src/main.rs`
- Create: `specs/puffer-cli/119.md`

- [ ] **Step 1: Add proxy client helpers**

In `crates/puffer-cli/src/daemon.rs`, add private helpers near `build_runtime_inputs_with_discovery`:

```rust
fn proxy_discovery_client(config: &PufferConfig) -> Result<puffer_provider_registry::ModelDiscoveryClient> {
    let client = puffer_core::blocking_client_for_url(
        &config.network.proxy,
        puffer_core::HttpPurpose::Discovery,
        "https://api.openai.com/v1/models",
        Duration::from_secs(8),
    )?;
    Ok(puffer_provider_registry::ModelDiscoveryClient::with_client(client))
}

fn proxy_oauth_client(config: &PufferConfig, url: &str) -> Result<reqwest::blocking::Client> {
    puffer_core::blocking_client_for_url(
        &config.network.proxy,
        puffer_core::HttpPurpose::OAuth,
        url,
        Duration::from_secs(60),
    )
}
```

In `crates/puffer-cli/src/main.rs`, add equivalent private helpers near `run_auth_command`:

```rust
fn proxy_discovery_client(config: &puffer_config::PufferConfig) -> Result<puffer_provider_registry::ModelDiscoveryClient> {
    let client = puffer_core::blocking_client_for_url(
        &config.network.proxy,
        puffer_core::HttpPurpose::Discovery,
        "https://api.openai.com/v1/models",
        Duration::from_secs(8),
    )?;
    Ok(puffer_provider_registry::ModelDiscoveryClient::with_client(client))
}

fn proxy_oauth_client(config: &puffer_config::PufferConfig, url: &str) -> Result<reqwest::blocking::Client> {
    puffer_core::blocking_client_for_url(
        &config.network.proxy,
        puffer_core::HttpPurpose::OAuth,
        url,
        Duration::from_secs(60),
    )
}
```

- [ ] **Step 2: Wire daemon discovery**

In `build_runtime_inputs_with_discovery`, replace:

```rust
let _ = providers.discover_and_merge_all(&auth_store);
```

with:

```rust
if let Ok(client) = proxy_discovery_client(&config) {
    let _ = providers.discover_and_merge_all_with_discovery_client(&auth_store, &client);
} else {
    let _ = providers.discover_and_merge_all(&auth_store);
}
```

In `handle_list_provider_models`, use `discover_and_merge_provider_with_discovery_client` with the current config.

- [ ] **Step 3: Wire daemon OAuth**

In `handle_login_with_oauth`, replace OpenAI exchange with:

```rust
let oauth_client = proxy_oauth_client(&state.config.lock().unwrap().clone(), puffer_provider_openai::OPENAI_TOKEN_URL)?;
let credential = puffer_provider_openai::exchange_authorization_code_with_client(
    &oauth_client,
    &code,
    &bundle.verifier,
    None,
)?;
```

Replace Anthropic exchange with:

```rust
let oauth_client = proxy_oauth_client(&state.config.lock().unwrap().clone(), ANTHROPIC_API_BASE_URL)?;
let credential = puffer_transport_anthropic::exchange_authorization_code_with_client(
    &oauth_client,
    &code,
    &bundle.verifier,
    &bundle.state,
    Some(&redirect_uri),
    Some(ANTHROPIC_API_BASE_URL),
)?;
```

- [ ] **Step 4: Wire CLI startup discovery and auth refresh**

At the startup discovery block currently shaped as:

```rust
let stale_provider_ids = providers.apply_discovery_cache();
let _discovery_handle = providers.start_background_discovery(stale_provider_ids, &auth_store);
```

replace the second line with proxy-aware background discovery:

```rust
let _discovery_handle = if let Ok(client) = proxy_discovery_client(&config) {
    providers.start_background_discovery_with_discovery_client(
        stale_provider_ids,
        &auth_store,
        client,
    )
} else {
    providers.start_background_discovery(stale_provider_ids, &auth_store)
};
```

This call uses the Task 3 `ProviderRegistry::start_background_discovery_with_discovery_client` implementation.

In `run_auth_command`, replace direct OpenAI exchange:

```rust
let credential = exchange_openai_code(&code, &verifier, None)?;
```

with:

```rust
let credential = match proxy_oauth_client(config, puffer_provider_openai::OPENAI_TOKEN_URL) {
    Ok(client) => puffer_provider_openai::exchange_authorization_code_with_client(
        &client,
        &code,
        &verifier,
        None,
    )?,
    Err(_) => exchange_openai_code(&code, &verifier, None)?,
};
```

Replace direct Anthropic exchange:

```rust
let credential = exchange_anthropic_code(
    &code,
    &verifier,
    &expected_state,
    Some(&redirect_uri),
    Some(ANTHROPIC_API_BASE_URL),
)?;
```

with:

```rust
let credential = match proxy_oauth_client(config, ANTHROPIC_API_BASE_URL) {
    Ok(client) => puffer_transport_anthropic::exchange_authorization_code_with_client(
        &client,
        &code,
        &verifier,
        &expected_state,
        Some(&redirect_uri),
        Some(ANTHROPIC_API_BASE_URL),
    )?,
    Err(_) => exchange_anthropic_code(
        &code,
        &verifier,
        &expected_state,
        Some(&redirect_uri),
        Some(ANTHROPIC_API_BASE_URL),
    )?,
};
```

Apply the same pattern in `run_login_flow`. Change its signature to accept `config: &puffer_config::PufferConfig`, update the `AuthCommand::Login` caller, and use `bundle.state` in the Anthropic branch.

For `AuthCommand::OauthRefresh`, replace:

```rust
refresh_openai_oauth_token(&existing.refresh_token)?
```

with:

```rust
match proxy_oauth_client(config, puffer_provider_openai::OPENAI_TOKEN_URL) {
    Ok(client) => puffer_provider_openai::refresh_oauth_token_with_client(
        &client,
        &existing.refresh_token,
    )?,
    Err(_) => refresh_openai_oauth_token(&existing.refresh_token)?,
}
```

Replace:

```rust
refresh_anthropic_oauth_token(
    &existing.refresh_token,
    anthropic_refresh_scopes(existing).as_deref(),
    Some(ANTHROPIC_API_BASE_URL),
    Some(&registry_to_anthropic_oauth_credential(existing)),
)?
```

with:

```rust
match proxy_oauth_client(config, ANTHROPIC_API_BASE_URL) {
    Ok(client) => puffer_transport_anthropic::refresh_oauth_token_with_client(
        &client,
        &existing.refresh_token,
        anthropic_refresh_scopes(existing).as_deref(),
        Some(&registry_to_anthropic_oauth_credential(existing)),
    )?,
    Err(_) => refresh_anthropic_oauth_token(
        &existing.refresh_token,
        anthropic_refresh_scopes(existing).as_deref(),
        Some(ANTHROPIC_API_BASE_URL),
        Some(&registry_to_anthropic_oauth_credential(existing)),
    )?,
}
```

- [ ] **Step 5: Run daemon/CLI tests**

```bash
cargo test -p puffer-cli list_provider_models_uses_fresh_discovery_over_static_models -- --nocapture
cargo test -p puffer-cli oauth -- --nocapture
cargo test -p puffer-cli settings_snapshot -- --nocapture
cargo test -p puffer-cli list_provider_models -- --nocapture
```

Expected: pass. If a renamed local test is needed, use the exact test name from `cargo test -p puffer-cli -- --list | rg 'oauth|provider_models|settings'`.

- [ ] **Step 6: Add component update spec**

Create `specs/puffer-cli/119.md`:

```markdown
# 119 - Provider Proxy Daemon Wiring

Routes daemon and CLI provider discovery/OAuth through proxy-aware blocking clients when configured.

- Daemon runtime input building uses injected provider-discovery clients.
- Provider model listing refreshes through the same proxy-aware discovery path.
- OAuth login and refresh paths can use injected proxy-aware clients.
- Existing non-proxy behavior remains the fallback when proxy is disabled or client construction fails.
```

- [ ] **Step 7: Commit daemon/CLI wiring**

```bash
git add crates/puffer-cli/src/daemon.rs crates/puffer-cli/src/main.rs specs/puffer-cli/119.md
git commit -m "feat(cli): wire provider proxy into daemon networking"
```

## Task 7: Desktop Settings API

**Files:**
- Modify: `crates/puffer-cli/src/desktop_api_types.rs`
- Modify: `crates/puffer-cli/src/desktop_api.rs`
- Modify: `crates/puffer-cli/src/daemon.rs`
- Modify: `apps/puffer-desktop/src/lib/types.ts`
- Modify: `apps/puffer-desktop/src/lib/api/desktop.ts`

- [ ] **Step 1: Add DTOs**

In `desktop_api_types.rs`, add:

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetworkProxySettingsDto {
    pub(crate) enabled: bool,
    pub(crate) selected: Option<String>,
    pub(crate) bypass: Vec<String>,
    pub(crate) proxies: Vec<SanitizedProxyEndpointDto>,
    pub(crate) last_test: Option<ProxyTestResultDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SanitizedProxyEndpointDto {
    pub(crate) id: String,
    pub(crate) scheme: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: Option<String>,
    pub(crate) has_password: bool,
    pub(crate) uri: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyTestResultDto {
    pub(crate) proxy_id: Option<String>,
    pub(crate) ok: bool,
    pub(crate) message: String,
    pub(crate) latency_ms: Option<u128>,
}
```

Add `pub(crate) network_proxy: NetworkProxySettingsDto` to `SettingsSnapshotDto`.

Add request DTOs for saving edited proxy state. These are separate from the sanitized snapshot so password preservation is explicit:

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveProxySettingsParams {
    pub(crate) enabled: bool,
    pub(crate) selected: Option<String>,
    pub(crate) bypass: Vec<String>,
    pub(crate) proxies: Vec<ProxyEndpointInputDto>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyEndpointInputDto {
    pub(crate) id: String,
    pub(crate) scheme: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) keep_password: bool,
}
```

- [ ] **Step 2: Populate settings snapshot**

In `desktop_api.rs`, build `network_proxy` from `config.network.proxy`, using `endpoint.sanitized()` and stringifying `ProxyScheme::as_uri_scheme()`.

- [ ] **Step 3: Add daemon RPC handlers**

In `daemon.rs`, register:

```rust
"save_proxy_settings" => respond!(detached!(|s, p| handle_save_proxy_settings(&s, &p))),
"test_proxy" => respond!(detached!(|s, p| handle_test_proxy(&s, &p))),
```

Implement:

```rust
fn handle_save_proxy_settings(state: &DaemonState, params: &Value) -> Result<Value>
```

Parse `SaveProxySettingsParams` from JSON, convert it into `ProxyConfig`, call `validate`, save user config, reload daemon config, and return a fresh `SettingsSnapshotDto`.

When `ProxyEndpointInputDto.keep_password` is true and `password` is `None`, preserve the current stored password for the matching proxy id:

```rust
let existing_password = current_config
    .network
    .proxy
    .proxies
    .iter()
    .find(|endpoint| endpoint.id == input.id)
    .and_then(|endpoint| endpoint.password.clone());
let password = input.password.or_else(|| input.keep_password.then_some(existing_password).flatten());
```

Do not return passwords in the snapshot.

Implement:

```rust
fn handle_test_proxy(state: &DaemonState, params: &Value) -> Result<Value>
```

Support `{ "proxyId": "..." }` and `{ "endpoint": { ... } }`. Resolve the target URL with this deterministic helper:

```rust
fn proxy_test_target_url(providers: &ProviderRegistry, config: &PufferConfig) -> String {
    let provider_id = config
        .default_provider
        .as_deref()
        .unwrap_or("openai");
    if let Some(provider) = providers.provider(provider_id) {
        if let Some(discovery) = provider.discovery.as_ref() {
            let base = provider.base_url.trim_end_matches('/');
            let path = if base.ends_with("/v1") && discovery.path.starts_with("/v1/") {
                discovery.path[3..].to_string()
            } else {
                discovery.path.clone()
            };
            return format!("{base}{path}");
        }
        return provider.base_url.clone();
    }
    "https://api.openai.com/v1/models".to_string()
}
```

Call `puffer_core::test_proxy_endpoint` with a 5 to 8 second timeout and return `ProxyTestResultDto`. Error results should set `ok: false`, include a short message, and must not include proxy passwords.

- [ ] **Step 4: Add frontend types**

In `apps/puffer-desktop/src/lib/types.ts`, add:

```ts
export type ProxyScheme = "http" | "https" | "socks5" | "socks5h";

export type SanitizedProxyEndpoint = {
  id: string;
  scheme: ProxyScheme;
  host: string;
  port: number;
  username: string | null;
  hasPassword: boolean;
  uri: string;
};

export type ProxyTestResult = {
  proxyId: string | null;
  ok: boolean;
  message: string;
  latencyMs: number | null;
};

export type NetworkProxySettings = {
  enabled: boolean;
  selected: string | null;
  bypass: string[];
  proxies: SanitizedProxyEndpoint[];
  lastTest: ProxyTestResult | null;
};

export type DraftProxyEndpoint = {
  id: string;
  scheme: ProxyScheme;
  host: string;
  port: number;
  username?: string | null;
  password?: string | null;
  keepPassword?: boolean;
};

export type SaveProxySettingsInput = {
  enabled: boolean;
  selected: string | null;
  bypass: string[];
  proxies: DraftProxyEndpoint[];
};
```

Add `networkProxy: NetworkProxySettings` to `SettingsSnapshot`.

- [ ] **Step 5: Add frontend API functions**

In `desktop.ts`, import `DraftProxyEndpoint`, `ProxyTestResult`, `SaveProxySettingsInput`, and `SettingsSnapshot` from `types.ts`, then add:

```ts
export async function saveProxySettings(input: SaveProxySettingsInput): Promise<SettingsSnapshot> {
  const client = await ensureLocalDaemonClient();
  return client.request<SettingsSnapshot>("save_proxy_settings", input);
}

export async function testProxy(input: { proxyId?: string; endpoint?: DraftProxyEndpoint }): Promise<ProxyTestResult> {
  const client = await ensureLocalDaemonClient();
  return client.request<ProxyTestResult>("test_proxy", input);
}
```

- [ ] **Step 6: Run API tests/checks**

```bash
cargo test -p puffer-cli settings -- --nocapture
cd apps/puffer-desktop && npm run check
```

Expected: pass after adding required mock fields.

- [ ] **Step 7: Commit desktop API**

```bash
git add crates/puffer-cli/src/desktop_api_types.rs crates/puffer-cli/src/desktop_api.rs crates/puffer-cli/src/daemon.rs apps/puffer-desktop/src/lib/types.ts apps/puffer-desktop/src/lib/api/desktop.ts
git commit -m "feat(desktop): expose proxy settings API"
```

## Task 8: Settings Network UI

**Files:**
- Create: `apps/puffer-desktop/src/lib/screens/settings/NetworkSettings.svelte`
- Modify: `apps/puffer-desktop/src/lib/screens/Settings.svelte`
- Modify: `apps/puffer-desktop/src/lib/mockData.ts`
- Create: `specs/puffer-desktop/586.md`

- [ ] **Step 1: Create NetworkSettings component**

Create `NetworkSettings.svelte` with props:

```svelte
<script lang="ts">
  import type { NetworkProxySettings, ProxyScheme, SettingsSnapshot } from "../../types";
  import { saveProxySettings, testProxy, type DraftProxyEndpoint } from "../../api/desktop";
  import Icon from "../../design/Icon.svelte";

  export let snapshot: SettingsSnapshot | null;
  export let onSaved: (snapshot: SettingsSnapshot) => void;

  let saving = false;
  let testingId: string | null = null;
  let error: string | null = null;
  let draftPassword = "";
  let editing: DraftProxyEndpoint | null = null;

  $: proxy = snapshot?.networkProxy ?? {
    enabled: false,
    selected: null,
    bypass: ["localhost", "127.0.0.1", "::1", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"],
    proxies: [],
    lastTest: null
  };

  function nextProxyId() {
    return `proxy-${Date.now()}`;
  }

  function toSaveProxySettingsInput(next: NetworkProxySettings) {
    return {
      enabled: next.enabled,
      selected: next.selected,
      bypass: next.bypass,
      proxies: next.proxies.map((item) => ({
        id: item.id,
        scheme: item.scheme,
        host: item.host,
        port: item.port,
        username: item.username,
        password: null,
        keepPassword: item.hasPassword
      }))
    };
  }

  async function persist(next: NetworkProxySettings) {
    saving = true;
    error = null;
    try {
      const input = toSaveProxySettingsInput(next);
      onSaved(await saveProxySettings(input));
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      saving = false;
    }
  }
</script>
```

Render the pane with the current Settings grammar: `h2`, `.lead`, `.pf-settings-row`, `.meta`, `.label`, `.desc`, `.sc-switch`, `.sc-input`, `.sc-btn`, `.pf-settings-note`, and `.pf-empty`. Add only component-local classes for the card list and modal.

Use this top-level structure:

```svelte
<h2>Network</h2>
<p class="lead">Provider proxy configuration for model, discovery, and OAuth requests.</p>

{#if error}
  <div class="pf-settings-note warn">{error}</div>
{/if}

<div class="pf-settings-row">
  <div class="meta">
    <div class="label">Proxy</div>
    <div class="desc">Route provider traffic through the selected proxy.</div>
  </div>
  <input
    type="checkbox"
    class="sc-switch"
    checked={proxy.enabled}
    onchange={(e) => persist({ ...proxy, enabled: (e.currentTarget as HTMLInputElement).checked })}
  />
</div>

<section class="pf-network-section" aria-label="Proxy list">
  <div class="pf-network-section-head">
    <div>
      <h3>Proxy list</h3>
      <p>Saved endpoints available for provider requests.</p>
    </div>
  </div>
  <div class="pf-network-list">
    {#if proxy.proxies.length === 0}
      <div class="pf-empty">No proxies added.</div>
    {:else}
      {#each proxy.proxies as item (item.id)}
        <article class="pf-network-proxy-card" data-selected={proxy.selected === item.id}>
          <label class="pf-network-proxy-main">
            <input
              type="radio"
              name="proxy"
              checked={proxy.selected === item.id}
              onchange={() => persist({ ...proxy, selected: item.id })}
            />
            <span>
              <strong>{item.uri}</strong>
              {#if item.username}
                <small>{item.username}</small>
              {/if}
            </span>
          </label>
          <div class="pf-network-proxy-actions">
            <button type="button" class="sc-btn" data-variant="outline" data-size="sm" onclick={() => testSavedProxy(item.id)}>
              <Icon name="test" size={12} />{testingId === item.id ? "Testing..." : "Test"}
            </button>
            <button type="button" class="sc-btn" data-variant="outline" data-size="sm" onclick={() => editProxy(item)}>
              <Icon name="edit" size={12} />Edit
            </button>
          </div>
        </article>
      {/each}
    {/if}
  </div>
  <button type="button" class="sc-btn pf-network-add" data-variant="outline" data-size="sm" onclick={addProxy}>
    <Icon name="plus" size={12} />Add proxy
  </button>
</section>

<section class="pf-network-section" aria-label="Bypass">
  <div class="pf-network-section-head">
    <div>
      <h3>Bypass</h3>
      <p>Hosts, IPs, and CIDR ranges that should skip the proxy.</p>
    </div>
  </div>
  <textarea class="sc-input pf-network-bypass" bind:value={bypassDraft}></textarea>
  <div class="pf-network-actions">
    <button type="button" class="sc-btn" data-variant="outline" data-size="sm" onclick={resetBypassDefaults}>
      Reset defaults
    </button>
    <button type="button" class="sc-btn" data-size="sm" disabled={saving} onclick={saveBypass}>
      Save bypass
    </button>
  </div>
</section>

<section class="pf-network-section" aria-label="Connectivity check">
  <div class="pf-network-section-head">
    <div>
      <h3>Connectivity check</h3>
      <p>Tests the selected proxy against provider network targets.</p>
    </div>
  </div>
  <button type="button" class="sc-btn" data-variant="outline" data-size="sm" disabled={!proxy.selected} onclick={testSelectedProxy}>
    <Icon name="test" size={12} />Test selected proxy
  </button>
  {#if proxy.lastTest}
    <div class="pf-settings-note" data-ok={proxy.lastTest.ok}>{proxy.lastTest.message}</div>
  {/if}
</section>
```

Implement local functions `addProxy`, `editProxy`, `testSavedProxy`, `testSelectedProxy`, `saveBypass`, and `resetBypassDefaults` in the same component. The card must not show a Ready badge, and no control may mention or imply `All Puffer HTTP`.

- [ ] **Step 2: Wire Settings navigation**

In `Settings.svelte`, update:

```ts
type Section = "general" | "providers" | "network" | "connectors" | "permissions" | "skills" | "mcp" | "git" | "appearance" | "shortcuts";
```

Add nav item:

```ts
{ id: "network", label: "Network", icon: "server" }
```

Import `NetworkSettings` and render it when `section === "network"`:

```svelte
{:else if section === "network"}
  <NetworkSettings snapshot={props.snapshot} onSaved={(next) => props.onRefresh()} />
```

Render returned snapshots by calling the supplied `onSaved(next)` callback. The parent can choose to refresh from the daemon after save; the component should not create a landing/help page.

- [ ] **Step 3: Add mock data**

In `mockData.ts`, add `networkProxy` to `mockSettingsSnapshot`:

```ts
networkProxy: {
  enabled: false,
  selected: null,
  bypass: ["localhost", "127.0.0.1", "::1", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"],
  proxies: [],
  lastTest: null
}
```

- [ ] **Step 4: Run desktop check**

```bash
cd apps/puffer-desktop && npm run check
```

Expected: pass.

- [ ] **Step 5: Manual UI verification**

Run local desktop dev server if not already running:

```bash
cd apps/puffer-desktop && npm run dev
```

Open `http://127.0.0.1:1420`, navigate Settings -> Network, and verify:

- empty proxy list state renders;
- Add proxy opens modal;
- proxy card shows URI without password;
- Test and Edit are inside card;
- Add proxy sits below list and left aligned;
- Bypass title/description are above textarea;
- Save bypass is visible;
- there is no active `All Puffer HTTP` control.

- [ ] **Step 6: Add component update spec**

Create `specs/puffer-desktop/586.md`:

```markdown
# 586 - Network Proxy Settings UI

Adds the Settings -> Network pane for phase 1 provider proxy configuration.

- Uses existing Settings layout grammar with rows, full-width list sections, and card-local actions.
- Allows users to add, edit, select, test, and save provider proxies.
- Shows proxy URIs without credentials and preserves password values through placeholder behavior.
- Adds Bypass as title/description plus textarea and Save action.
- Does not expose All Puffer HTTP controls in phase 1.
```

- [ ] **Step 7: Commit Settings UI**

```bash
git add apps/puffer-desktop/src/lib/screens/settings/NetworkSettings.svelte apps/puffer-desktop/src/lib/screens/Settings.svelte apps/puffer-desktop/src/lib/mockData.ts specs/puffer-desktop/586.md
git commit -m "feat(desktop): add network proxy settings UI"
```

## Task 9: End-To-End Verification

**Files:**
- No required source files.
- Optional modify: tests if a local proxy stub is added during implementation.

- [ ] **Step 1: Run focused Rust tests**

```bash
cargo test -p puffer-config proxy_config -- --nocapture
cargo test -p puffer-core network::tests -- --nocapture
cargo test -p puffer-provider-registry discovery_client_accepts_injected_reqwest_client -- --nocapture
cargo test -p puffer-provider-openai refresh_oauth_token -- --nocapture
cargo test -p puffer-transport-anthropic oauth -- --nocapture
cargo test -p puffer-cli list_provider_models_uses_fresh_discovery_over_static_models -- --nocapture
```

Expected: all pass.

- [ ] **Step 2: Run desktop validation**

```bash
cd apps/puffer-desktop && npm run check && npm run build
```

Expected: both pass.

- [ ] **Step 3: Run broader workspace gate**

```bash
cargo test --workspace
```

Expected: pass. If this is too slow for the local machine, record the focused test results and the reason the full workspace gate was skipped.

- [ ] **Step 4: Manual proxy verification with local proxy**

Start a known local proxy such as SOCKS5 on `127.0.0.1:7890`. In desktop Settings -> Network:

1. Add proxy `SOCKS5H 127.0.0.1 7890`.
2. Enable Proxy.
3. Select the proxy.
4. Click Test.
5. Send a WorldRouter/OpenAI-compatible chat message.
6. Confirm the model returns.
7. Disable Proxy.
8. Send another message and confirm direct path still works.

Expected: both proxied and direct sends work; no proxy password appears in UI, daemon logs, or error messages.

- [ ] **Step 5: Record verification results**

```bash
git status --short
```

If verification creates a tracked proxy integration test, commit only that file:

```bash
git add crates/puffer-core/runtime/tests/proxy_http.rs
git commit -m "test: verify provider proxy flow"
```

If that file does not exist or verification creates no tracked changes, do not create an empty commit.

## Self-Review Notes

- Spec coverage: config, network helper, discovery, OAuth, model sends, daemon API, UI, testing, and Phase 2 exclusions are all mapped to tasks.
- Scope check: plan stays inside phase 1 provider proxy; broader HTTP routing remains out of scope.
- Placeholder scan: no `TODO`, `TBD`, or undefined "fill in later" steps are intended.
- Type consistency: `ProxyConfig`, `ProxyEndpoint`, `ProxyScheme`, `HttpPurpose`, `NetworkProxySettings`, `save_proxy_settings`, and `test_proxy` are introduced before use.
