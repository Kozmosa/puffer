//! CDP network request tracking for BrowserAction network-idle waits.

use serde_json::Value;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use url::Url;

/// Tracks active CDP network requests for one browser page worker.
#[derive(Debug)]
pub(super) struct BrowserNetworkState {
    active_request_ids: HashSet<String>,
    last_activity: Instant,
}

impl Default for BrowserNetworkState {
    fn default() -> Self {
        Self {
            active_request_ids: HashSet::new(),
            last_activity: Instant::now(),
        }
    }
}

impl BrowserNetworkState {
    /// Applies one CDP network event to the active request set.
    pub(super) fn update_from_cdp(&mut self, method: &str, value: &Value) {
        let Some(request_id) = value
            .pointer("/params/requestId")
            .and_then(Value::as_str)
            .map(ToString::to_string)
        else {
            return;
        };
        match method {
            "Network.requestWillBeSent" => {
                if should_track_request(value) {
                    self.active_request_ids.insert(request_id);
                    self.last_activity = Instant::now();
                } else if self.active_request_ids.remove(&request_id) {
                    self.last_activity = Instant::now();
                }
            }
            "Network.loadingFinished" | "Network.loadingFailed" => {
                if self.active_request_ids.remove(&request_id) {
                    self.last_activity = Instant::now();
                }
            }
            _ => {}
        }
    }

    /// Returns true when no tracked request has been active for `idle`.
    pub(super) fn is_idle_for(&self, idle: Duration) -> bool {
        self.active_request_ids.is_empty() && self.last_activity.elapsed() >= idle
    }

    /// Returns the number of currently tracked active requests.
    pub(super) fn active_count(&self) -> usize {
        self.active_request_ids.len()
    }
}

fn should_track_request(value: &Value) -> bool {
    let request_type = value
        .pointer("/params/type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if is_non_blocking_resource_type(request_type) {
        return false;
    }
    let method = value
        .pointer("/params/request/method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if method.eq_ignore_ascii_case("OPTIONS") {
        return false;
    }
    let Some(url) = value.pointer("/params/request/url").and_then(Value::as_str) else {
        return true;
    };
    is_meaningful_network_url(url)
}

fn is_non_blocking_resource_type(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "beacon"
            | "cspviolationreport"
            | "eventsource"
            | "ping"
            | "preflight"
            | "reporting"
            | "websocket"
    )
}

fn is_meaningful_network_url(value: &str) -> bool {
    let Ok(parsed) = Url::parse(value) else {
        return true;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let path = parsed.path().to_ascii_lowercase();
    let query = parsed.query().unwrap_or_default().to_ascii_lowercase();
    if known_telemetry_host(&host) {
        return false;
    }
    if known_telemetry_path(&path) {
        return false;
    }
    if query.contains("collect") && query.contains("analytics") {
        return false;
    }
    true
}

fn known_telemetry_host(host: &str) -> bool {
    const MARKERS: &[&str] = &[
        "amplitude",
        "analytics.google",
        "analytics.shopify",
        "clarity.ms",
        "connect.facebook",
        "datadog",
        "doubleclick",
        "facebook.com",
        "fullstory",
        "google-analytics",
        "googletagmanager",
        "heap.io",
        "hotjar",
        "intercom",
        "klaviyo",
        "mixpanel",
        "monorail",
        "pinterest",
        "posthog",
        "rudderstack",
        "segment.io",
        "sentry",
        "shopifysvc",
        "snapchat",
        "stats.g.doubleclick",
        "tiktok",
    ];
    MARKERS.iter().any(|marker| host.contains(marker))
}

fn known_telemetry_path(path: &str) -> bool {
    const MARKERS: &[&str] = &[
        "/analytics",
        "/beacon",
        "/collect",
        "/metrics",
        "/mp/collect",
        "/rum",
        "/session-replay",
        "/telemetry",
        "/traces",
        "/vitals",
    ];
    MARKERS.iter().any(|marker| path.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tracks_active_request_ids() {
        let mut network = BrowserNetworkState::default();

        network.update_from_cdp(
            "Network.requestWillBeSent",
            &json!({
                "params": {
                    "requestId": "r1",
                    "type": "Fetch",
                    "request": {
                        "method": "GET",
                        "url": "https://checkout.example.com/api/cart"
                    }
                }
            }),
        );
        assert_eq!(network.active_count(), 1);

        network.update_from_cdp(
            "Network.loadingFinished",
            &json!({ "params": { "requestId": "r1" } }),
        );
        assert_eq!(network.active_count(), 0);
    }

    #[test]
    fn active_requests_are_not_idle() {
        let mut network = BrowserNetworkState::default();
        network.update_from_cdp(
            "Network.requestWillBeSent",
            &json!({
                "params": {
                    "requestId": "r1",
                    "type": "Fetch",
                    "request": {
                        "method": "GET",
                        "url": "https://checkout.example.com/api/cart"
                    }
                }
            }),
        );

        assert!(!network.is_idle_for(Duration::from_millis(0)));
    }

    #[test]
    fn ignores_beacon_and_preflight_requests() {
        let mut network = BrowserNetworkState::default();

        network.update_from_cdp(
            "Network.requestWillBeSent",
            &json!({
                "params": {
                    "requestId": "beacon",
                    "type": "Ping",
                    "request": {
                        "method": "POST",
                        "url": "https://checkout.example.com/track"
                    }
                }
            }),
        );
        network.update_from_cdp(
            "Network.requestWillBeSent",
            &json!({
                "params": {
                    "requestId": "preflight",
                    "type": "Preflight",
                    "request": {
                        "method": "OPTIONS",
                        "url": "https://checkout.example.com/api/cart"
                    }
                }
            }),
        );

        assert_eq!(network.active_count(), 0);
        assert!(network.is_idle_for(Duration::from_millis(0)));
    }

    #[test]
    fn ignores_known_telemetry_hosts_and_paths() {
        let mut network = BrowserNetworkState::default();

        network.update_from_cdp(
            "Network.requestWillBeSent",
            &json!({
                "params": {
                    "requestId": "ga",
                    "type": "Fetch",
                    "request": {
                        "method": "POST",
                        "url": "https://www.google-analytics.com/g/collect?v=2"
                    }
                }
            }),
        );
        network.update_from_cdp(
            "Network.requestWillBeSent",
            &json!({
                "params": {
                    "requestId": "rum",
                    "type": "XHR",
                    "request": {
                        "method": "POST",
                        "url": "https://checkout.example.com/rum/events"
                    }
                }
            }),
        );

        assert_eq!(network.active_count(), 0);
    }

    #[test]
    fn keeps_unknown_fetch_requests_active() {
        let mut network = BrowserNetworkState::default();

        network.update_from_cdp(
            "Network.requestWillBeSent",
            &json!({
                "params": {
                    "requestId": "cart",
                    "type": "Fetch",
                    "request": {
                        "method": "POST",
                        "url": "https://checkout.example.com/api/cart"
                    }
                }
            }),
        );

        assert_eq!(network.active_count(), 1);
    }

    #[test]
    fn ignored_redirect_releases_previous_meaningful_request() {
        let mut network = BrowserNetworkState::default();

        network.update_from_cdp(
            "Network.requestWillBeSent",
            &json!({
                "params": {
                    "requestId": "r1",
                    "type": "Fetch",
                    "request": {
                        "method": "GET",
                        "url": "https://checkout.example.com/api/cart"
                    }
                }
            }),
        );
        network.update_from_cdp(
            "Network.requestWillBeSent",
            &json!({
                "params": {
                    "requestId": "r1",
                    "type": "Ping",
                    "request": {
                        "method": "POST",
                        "url": "https://checkout.example.com/beacon"
                    }
                }
            }),
        );

        assert_eq!(network.active_count(), 0);
    }
}
