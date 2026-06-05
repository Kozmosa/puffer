use crate::dtos::MediaCapabilityInfoDto;
use serde_json::json;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const CAPABILITY_CACHE_TTL_MS: u64 = 30_000;

#[derive(Debug, Clone)]
struct CachedCapabilities {
    openai_connected: bool,
    checked_at_ms: u64,
    capabilities: Vec<MediaCapabilityInfoDto>,
}

/// Caches deterministic media capability discovery for the desktop backend.
#[derive(Debug, Default)]
pub(crate) struct MediaCapabilityCache {
    cached: Mutex<Option<CachedCapabilities>>,
}

impl MediaCapabilityCache {
    /// Creates an empty media capability cache.
    pub(crate) fn new() -> Self {
        Self {
            cached: Mutex::new(None),
        }
    }

    /// Lists cached or freshly discovered capabilities, optionally filtered by media kind.
    pub(crate) fn list(
        &self,
        openai_connected: bool,
        kind: Option<&str>,
    ) -> Vec<MediaCapabilityInfoDto> {
        let now = now_ms();
        let capabilities = {
            let mut cached = self
                .cached
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let usable = cached.as_ref().is_some_and(|entry| {
                entry.openai_connected == openai_connected
                    && now.saturating_sub(entry.checked_at_ms) <= CAPABILITY_CACHE_TTL_MS
            });
            if usable {
                cached
                    .as_ref()
                    .map(|entry| entry.capabilities.clone())
                    .unwrap_or_default()
            } else {
                let discovered = discover_capabilities(openai_connected, now);
                *cached = Some(CachedCapabilities {
                    openai_connected,
                    checked_at_ms: now,
                    capabilities: discovered.clone(),
                });
                discovered
            }
        };
        filter_kind(capabilities, kind)
    }
}

fn discover_capabilities(openai_connected: bool, checked_at_ms: u64) -> Vec<MediaCapabilityInfoDto> {
    if !openai_connected {
        return Vec::new();
    }
    vec![MediaCapabilityInfoDto {
        provider_id: "openai".to_string(),
        model_id: "gpt-image-1".to_string(),
        kind: "image".to_string(),
        operations: vec!["generate".to_string()],
        supports_async: false,
        supports_streaming: false,
        parameter_values: json!({
            "size": ["1024x1024", "1024x1536", "1536x1024"],
            "quality": ["auto", "low", "medium", "high"],
            "outputFormat": ["png", "jpeg", "webp"]
        }),
        status: "available".to_string(),
        source: "adapter:openai-images".to_string(),
        reason: None,
        checked_at_ms,
    }]
}

fn filter_kind(
    capabilities: Vec<MediaCapabilityInfoDto>,
    kind: Option<&str>,
) -> Vec<MediaCapabilityInfoDto> {
    let Some(kind) = kind.map(str::trim).filter(|value| !value.is_empty()) else {
        return capabilities;
    };
    capabilities
        .into_iter()
        .filter(|capability| capability.kind == kind)
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
