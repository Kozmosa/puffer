use crate::dtos::{MediaCapabilityInfoDto, MediaCapabilityParameterDto};
use puffer_core::{
    list_exact_media_capabilities_with_cache, ExactMediaDiscoveryCache, MediaCapabilityView,
};
use puffer_provider_registry::{AuthStore, ProviderRegistry};

/// Lists descriptor-backed exact media capabilities for the desktop backend.
pub(crate) fn list(
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    kind: Option<&str>,
    discovery_cache: &ExactMediaDiscoveryCache,
) -> Vec<MediaCapabilityInfoDto> {
    list_exact_media_capabilities_with_cache(providers, auth_store, kind, discovery_cache)
        .into_iter()
        .map(media_capability_info_dto)
        .collect()
}

fn media_capability_info_dto(capability: MediaCapabilityView) -> MediaCapabilityInfoDto {
    MediaCapabilityInfoDto {
        provider_id: capability.provider_id,
        provider_display_name: capability.provider_display_name,
        model_id: capability.model_id,
        model_display_name: capability.model_display_name,
        kind: capability.kind,
        operation: capability.operation,
        adapter: capability.adapter,
        parameters: capability
            .parameters
            .into_iter()
            .map(|parameter| MediaCapabilityParameterDto {
                name: parameter.name,
                label: parameter.label,
                values: parameter.values,
                default: parameter.default,
                request_field: parameter.request_field,
            })
            .collect(),
        defaults: capability.defaults,
        status: capability.status,
        source: capability.source,
        reason: capability.reason,
        checked_at_ms: capability.checked_at_ms,
    }
}
