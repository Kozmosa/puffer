use crate::dtos::{MediaCapabilityInfoDto, MediaCapabilityParameterDto};
use puffer_media::{
    list_exact_media_capabilities_with_cache, ExactMediaDiscoveryCache, MediaCapabilityView,
};
use puffer_provider_registry::{AuthStore, Axis, ControlKind, ProviderRegistry};
use std::collections::BTreeMap;

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
    let mut defaults = BTreeMap::new();
    let parameters = capability
        .axes
        .into_iter()
        .map(|axis| media_capability_parameter_dto(axis, &mut defaults))
        .collect();

    MediaCapabilityInfoDto {
        provider_id: capability.provider_id,
        provider_display_name: capability.provider_display_name,
        model_id: capability.model_id,
        model_display_name: capability.model_display_name,
        kind: capability.kind,
        operation: capability.operation,
        adapter: capability.adapter,
        parameters,
        defaults,
        status: capability.status,
        source: capability.source,
        reason: capability.reason,
        checked_at_ms: capability.checked_at_ms,
    }
}

fn media_capability_parameter_dto(
    axis: Axis,
    defaults: &mut BTreeMap<String, String>,
) -> MediaCapabilityParameterDto {
    let default = control_default(&axis.control);
    let values = control_values(&axis.control);
    defaults.insert(axis.id.clone(), default.clone());
    MediaCapabilityParameterDto {
        name: axis.id,
        label: axis.label,
        values,
        default,
        request_field: axis.request_field,
        wire_type: axis.wire_type.as_str().to_string(),
    }
}

fn control_default(control: &ControlKind) -> String {
    match control {
        ControlKind::Enum { default, .. } => default.clone(),
        ControlKind::Range { default, .. } => format_number(*default),
        ControlKind::Bool { default } => default.to_string(),
    }
}

fn control_values(control: &ControlKind) -> Vec<String> {
    match control {
        ControlKind::Enum { values, .. } => values.clone(),
        ControlKind::Range {
            min,
            max,
            step,
            default,
        } => range_values(*min, *max, *step, *default),
        ControlKind::Bool { .. } => vec!["false".to_string(), "true".to_string()],
    }
}

fn range_values(min: f64, max: f64, step: f64, default: f64) -> Vec<String> {
    if !min.is_finite() || !max.is_finite() || !step.is_finite() || step <= 0.0 || min > max {
        return vec![format_number(default)];
    }

    let max_steps = ((max - min) / step).floor();
    if max_steps > 128.0 {
        return vec![format_number(default)];
    }

    let mut values = Vec::new();
    let mut current = min;
    let epsilon = step.abs() * 1e-9;
    while current <= max + epsilon && values.len() <= 128 {
        push_unique(&mut values, format_number(current));
        current += step;
    }
    push_unique(&mut values, format_number(default));
    values
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn format_number(value: f64) -> String {
    format!("{value}")
}
