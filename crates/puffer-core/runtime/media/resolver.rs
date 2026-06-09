use super::capabilities::{MediaCapability, MediaCapabilityParameter, MediaKind};
use anyhow::{bail, Context, Result};
use puffer_provider_registry::{
    canonical_provider_id, AuthStore, MediaExecutionDescriptor, MediaExecutionKind,
    MediaModelDescriptor, MediaOperation, MediaParameterSpec, ProviderDescriptor, ProviderRegistry,
};
use std::collections::{BTreeMap, HashSet};

const CAPABILITY_STATUS_AVAILABLE: &str = "available";
const CAPABILITY_STATUS_UNAVAILABLE: &str = "unavailable";
const CAPABILITY_REASON_ADAPTER_UNAVAILABLE: &str = "adapter_unavailable";
const CAPABILITY_REASON_MISSING_AUTH: &str = "missing_auth";
const CAPABILITY_SOURCE_STATIC: &str = "static";

/// Carries cached dynamic media discovery records into capability resolution.
#[derive(Debug, Clone, Default)]
pub(crate) struct MediaDiscoveryCache {
    pub(crate) image_models: Vec<CachedImageMediaModel>,
}

/// Carries one cached exact image model discovered for a provider.
#[derive(Debug, Clone)]
pub(crate) struct CachedImageMediaModel {
    pub(crate) provider_id: String,
    pub(crate) model: MediaModelDescriptor,
    pub(crate) source: String,
}

/// Describes a saved exact image generation selection to validate.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ImageGenerationSelection<'a> {
    pub(crate) provider_id: &'a str,
    pub(crate) model_id: &'a str,
    pub(crate) adapter: &'a str,
    pub(crate) parameters: &'a BTreeMap<String, String>,
}

/// Describes a saved exact media generation selection to validate.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MediaGenerationSelection<'a> {
    pub(crate) kind: MediaKind,
    pub(crate) provider_id: &'a str,
    pub(crate) model_id: &'a str,
    pub(crate) operation: MediaOperation,
    pub(crate) adapter: &'a str,
    pub(crate) parameters: &'a BTreeMap<String, String>,
}

/// Resolves selectable exact media capabilities from provider descriptors.
pub(crate) fn resolve_media_capabilities(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    kind: MediaKind,
    operation: MediaOperation,
    checked_at_ms: u64,
    discovery_cache: &MediaDiscoveryCache,
) -> Vec<MediaCapability> {
    match kind {
        MediaKind::Image => resolve_image_capabilities(
            registry,
            auth_store,
            operation,
            checked_at_ms,
            discovery_cache,
        ),
        MediaKind::Video => {
            resolve_video_capabilities(registry, auth_store, operation, checked_at_ms)
        }
    }
}

/// Validates a saved exact image generation selection against current capabilities.
pub(crate) fn validate_image_generate_selection(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    selection: &ImageGenerationSelection<'_>,
    checked_at_ms: u64,
    discovery_cache: &MediaDiscoveryCache,
) -> Result<MediaCapability> {
    validate_media_generate_selection(
        registry,
        auth_store,
        &MediaGenerationSelection {
            kind: MediaKind::Image,
            provider_id: selection.provider_id,
            model_id: selection.model_id,
            operation: MediaOperation::Generate,
            adapter: selection.adapter,
            parameters: selection.parameters,
        },
        checked_at_ms,
        discovery_cache,
    )
}

/// Validates a saved exact media generation selection against current capabilities.
pub(crate) fn validate_media_generate_selection(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    selection: &MediaGenerationSelection<'_>,
    checked_at_ms: u64,
    discovery_cache: &MediaDiscoveryCache,
) -> Result<MediaCapability> {
    let capability = resolve_media_capabilities(
        registry,
        auth_store,
        selection.kind,
        selection.operation,
        checked_at_ms,
        discovery_cache,
    )
    .into_iter()
    .find(|capability| {
        capability.provider_id == selection.provider_id
            && capability.model_id == selection.model_id
            && capability.adapter == selection.adapter
    });

    let Some(capability) = capability else {
        bail!("{}", unavailable_selection_message(selection));
    };

    if capability.status != CAPABILITY_STATUS_AVAILABLE {
        bail!("{}", unavailable_selection_message(selection));
    }

    validate_parameter_values(selection.kind, &capability.parameters, selection.parameters)?;

    Ok(capability)
}

/// Resolves the provider and execution descriptor for a validated exact image selection.
pub(crate) fn resolve_image_execution_descriptor<'a>(
    registry: &'a ProviderRegistry,
    provider_id: &str,
    model_id: &str,
    adapter: &str,
    discovery_cache: &MediaDiscoveryCache,
) -> Result<(&'a ProviderDescriptor, MediaExecutionDescriptor)> {
    let unavailable =
        || format!("selected image model unavailable: {provider_id}/{model_id} via {adapter}");
    let provider = registry.provider(provider_id).with_context(unavailable)?;
    let image = provider
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .with_context(unavailable)?;
    let model = image
        .models
        .iter()
        .find(|model| model.id == model_id)
        .or_else(|| {
            discovery_cache
                .image_models
                .iter()
                .filter(|cached| cached.provider_id == provider.id)
                .map(|cached| &cached.model)
                .find(|model| model.id == model_id)
        })
        .with_context(unavailable)?;
    let execution = image_execution(image.execution.as_ref(), model).with_context(unavailable)?;
    if !execution_adapter_is_available_for_kind(MediaKind::Image, execution.adapter)
        || adapter_id(execution.adapter) != adapter
    {
        bail!("image media adapter unavailable for {adapter}");
    }

    Ok((provider, execution.clone()))
}

/// Resolves the provider and execution descriptor for a validated exact video selection.
pub(crate) fn resolve_video_execution_descriptor<'a>(
    registry: &'a ProviderRegistry,
    provider_id: &str,
    model_id: &str,
    adapter: &str,
) -> Result<(&'a ProviderDescriptor, MediaExecutionDescriptor)> {
    let unavailable =
        || format!("selected video model unavailable: {provider_id}/{model_id} via {adapter}");
    let provider = registry.provider(provider_id).with_context(unavailable)?;
    let video = provider
        .media
        .as_ref()
        .and_then(|media| media.video.as_ref())
        .with_context(unavailable)?;
    let model = video
        .models
        .iter()
        .find(|model| model.id == model_id)
        .with_context(unavailable)?;
    let execution = image_execution(video.execution.as_ref(), model).with_context(unavailable)?;
    if !execution_adapter_is_available_for_kind(MediaKind::Video, execution.adapter)
        || adapter_id(execution.adapter) != adapter
    {
        bail!("video media adapter unavailable for {adapter}");
    }

    Ok((provider, execution.clone()))
}

fn resolve_image_capabilities(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    operation: MediaOperation,
    checked_at_ms: u64,
    discovery_cache: &MediaDiscoveryCache,
) -> Vec<MediaCapability> {
    let mut capabilities = Vec::new();
    for provider in registry.providers() {
        if !provider_is_connected(provider, auth_store) {
            continue;
        }
        let Some(image) = provider
            .media
            .as_ref()
            .and_then(|media| media.image.as_ref())
        else {
            continue;
        };
        let mut emitted_model_ids = HashSet::new();
        let static_models = image
            .models
            .iter()
            .map(|model| (model, CAPABILITY_SOURCE_STATIC));
        let discovered_models = discovery_cache.image_models.iter().filter_map(|cached| {
            (cached.provider_id == provider.id).then_some((&cached.model, cached.source.as_str()))
        });
        for (model, source) in static_models.chain(discovered_models) {
            if !emitted_model_ids.insert(model.id.clone()) {
                continue;
            }
            if !media_model_is_available(model, operation) {
                continue;
            }
            let Some(execution) = image_execution(image.execution.as_ref(), model) else {
                continue;
            };
            if !execution_adapter_is_available_for_kind(MediaKind::Image, execution.adapter) {
                continue;
            }
            let parameters = media_parameters(model);
            capabilities.push(MediaCapability {
                provider_id: provider.id.clone(),
                provider_display_name: provider.display_name.clone(),
                model_id: model.id.clone(),
                model_display_name: model
                    .display_name
                    .clone()
                    .unwrap_or_else(|| model.id.clone()),
                kind: MediaKind::Image,
                operation: operation_wire_name(operation).to_string(),
                adapter: adapter_id(execution.adapter).to_string(),
                defaults: media_defaults(&parameters),
                parameters,
                status: CAPABILITY_STATUS_AVAILABLE.to_string(),
                source: source.to_string(),
                reason: None,
                checked_at_ms,
            });
        }
    }
    capabilities
}

fn resolve_video_capabilities(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    operation: MediaOperation,
    checked_at_ms: u64,
) -> Vec<MediaCapability> {
    let mut capabilities = Vec::new();
    for provider in registry.providers() {
        let Some(video) = provider
            .media
            .as_ref()
            .and_then(|media| media.video.as_ref())
        else {
            continue;
        };
        let provider_connected = provider_is_connected(provider, auth_store);
        for model in &video.models {
            if !media_model_is_available(model, operation) {
                continue;
            }
            let Some(execution) = image_execution(video.execution.as_ref(), model) else {
                continue;
            };
            let adapter_available =
                execution_adapter_is_available_for_kind(MediaKind::Video, execution.adapter);
            let (status, reason) = video_capability_state(provider_connected, adapter_available);
            let parameters = media_parameters(model);
            capabilities.push(MediaCapability {
                provider_id: provider.id.clone(),
                provider_display_name: provider.display_name.clone(),
                model_id: model.id.clone(),
                model_display_name: model
                    .display_name
                    .clone()
                    .unwrap_or_else(|| model.id.clone()),
                kind: MediaKind::Video,
                operation: operation_wire_name(operation).to_string(),
                adapter: adapter_id(execution.adapter).to_string(),
                defaults: media_defaults(&parameters),
                parameters,
                status: status.to_string(),
                source: CAPABILITY_SOURCE_STATIC.to_string(),
                reason: reason.map(str::to_string),
                checked_at_ms,
            });
        }
    }
    capabilities
}

fn unavailable_selection_message(selection: &MediaGenerationSelection<'_>) -> String {
    format!(
        "selected {} model unavailable: {}/{} via {}",
        media_kind_error_name(selection.kind),
        selection.provider_id,
        selection.model_id,
        selection.adapter
    )
}

fn video_capability_state(
    provider_connected: bool,
    adapter_available: bool,
) -> (&'static str, Option<&'static str>) {
    if !adapter_available {
        (
            CAPABILITY_STATUS_UNAVAILABLE,
            Some(CAPABILITY_REASON_ADAPTER_UNAVAILABLE),
        )
    } else if !provider_connected {
        (
            CAPABILITY_STATUS_UNAVAILABLE,
            Some(CAPABILITY_REASON_MISSING_AUTH),
        )
    } else {
        (CAPABILITY_STATUS_AVAILABLE, None)
    }
}

fn image_execution<'a>(
    provider_execution: Option<&'a MediaExecutionDescriptor>,
    model: &'a MediaModelDescriptor,
) -> Option<&'a MediaExecutionDescriptor> {
    model.execution.as_ref().or(provider_execution)
}

fn provider_is_connected(provider: &ProviderDescriptor, auth_store: &AuthStore) -> bool {
    if provider.auth_modes.is_empty() {
        return true;
    }
    if auth_store.has_auth(&provider.id) {
        return true;
    }
    let canonical = canonical_provider_id(&provider.id);
    if canonical != provider.id && auth_store.has_auth(&canonical) {
        return true;
    }
    provider.id == "openai" && auth_store.has_auth("codex")
}

fn execution_adapter_is_available_for_kind(kind: MediaKind, adapter: MediaExecutionKind) -> bool {
    matches!(
        (kind, adapter),
        (MediaKind::Image, MediaExecutionKind::ImagesJson)
            | (MediaKind::Image, MediaExecutionKind::ChatImageOutput)
            | (MediaKind::Image, MediaExecutionKind::MinimaxImage)
            | (MediaKind::Video, MediaExecutionKind::ReplicateVideo)
            | (MediaKind::Video, MediaExecutionKind::RelaydanceVideo)
            | (MediaKind::Video, MediaExecutionKind::BytePlusVideo)
    )
}

fn media_model_is_available(model: &MediaModelDescriptor, operation: MediaOperation) -> bool {
    let id = model.id.trim();
    !id.is_empty()
        && !id.eq_ignore_ascii_case("auto")
        && !has_wildcard_or_regex_marker(id)
        && model.operations.contains(&operation)
}

fn media_parameters(model: &MediaModelDescriptor) -> Vec<MediaCapabilityParameter> {
    model
        .parameters
        .iter()
        .map(parameter_from_descriptor)
        .collect()
}

fn parameter_from_descriptor(parameter: &MediaParameterSpec) -> MediaCapabilityParameter {
    MediaCapabilityParameter {
        name: parameter.name.clone(),
        label: parameter.label.clone(),
        values: parameter.values.clone(),
        default: parameter.default.clone(),
        request_field: parameter.request_field.clone(),
        wire_type: parameter.wire_type,
    }
}

fn media_defaults(parameters: &[MediaCapabilityParameter]) -> BTreeMap<String, String> {
    parameters
        .iter()
        .map(|parameter| (parameter.name.clone(), parameter.default.clone()))
        .collect()
}

fn validate_parameter_values(
    kind: MediaKind,
    parameters: &[MediaCapabilityParameter],
    selected: &BTreeMap<String, String>,
) -> Result<()> {
    for (name, value) in selected {
        let Some(parameter) = parameters.iter().find(|parameter| parameter.name == *name) else {
            bail!(
                "{} generation parameter unsupported: {name}={value}",
                media_kind_error_name(kind)
            );
        };
        if !parameter.values.iter().any(|candidate| candidate == value) {
            bail!(
                "{} generation parameter unsupported: {name}={value}",
                media_kind_error_name(kind)
            );
        }
    }
    Ok(())
}

fn operation_wire_name(operation: MediaOperation) -> &'static str {
    match operation {
        MediaOperation::Generate => "generate",
    }
}

fn media_kind_error_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
    }
}

pub(crate) fn adapter_id(adapter: MediaExecutionKind) -> &'static str {
    match adapter {
        MediaExecutionKind::ImagesJson => "images_json",
        MediaExecutionKind::ChatImageOutput => "chat_image_output",
        MediaExecutionKind::MinimaxImage => "minimax_image",
        MediaExecutionKind::ReplicateVideo => "replicate_video",
        MediaExecutionKind::RelaydanceVideo => "relaydance_video",
        MediaExecutionKind::BytePlusVideo => "byteplus_video",
    }
}

fn has_wildcard_or_regex_marker(value: &str) -> bool {
    value.chars().any(|ch| {
        matches!(
            ch,
            '*' | '?' | '[' | ']' | '(' | ')' | '{' | '}' | '|' | '^' | '$' | '\\'
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::media::MediaKind;
    use puffer_provider_registry::{
        AuthMode, AuthStore, MediaExecutionDescriptor, MediaExecutionKind, MediaKindDescriptor,
        MediaModelDescriptor, MediaOperation, MediaParameterSpec, MediaParameterWireType,
        ModelDescriptor, ProviderDescriptor, ProviderMediaDescriptor, ProviderRegistry,
    };

    fn registry_with(providers: Vec<ProviderDescriptor>) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register_many(providers);
        registry
    }

    fn provider(
        id: &str,
        auth_modes: Vec<AuthMode>,
        media: Option<ProviderMediaDescriptor>,
    ) -> ProviderDescriptor {
        ProviderDescriptor {
            id: id.to_string(),
            display_name: id.to_string(),
            base_url: format!("https://{id}.example"),
            default_api: "openai-responses".to_string(),
            auth_modes,
            headers: Default::default(),
            query_params: Default::default(),
            chat_completions_path: None,
            discovery: None,
            media,
            models: Vec::<ModelDescriptor>::new(),
        }
    }

    fn image_media(model_id: &str) -> ProviderMediaDescriptor {
        ProviderMediaDescriptor {
            image: Some(MediaKindDescriptor {
                discovery: None,
                execution: Some(MediaExecutionDescriptor {
                    adapter: MediaExecutionKind::ImagesJson,
                    base_url: None,
                    path: "/v1/images/generations".to_string(),
                    batch: puffer_provider_registry::MediaBatchDescriptor::default(),
                }),
                models: vec![MediaModelDescriptor {
                    id: model_id.to_string(),
                    display_name: Some("Display Image".to_string()),
                    execution: None,
                    operations: vec![MediaOperation::Generate],
                    parameters: vec![
                        MediaParameterSpec {
                            name: "size".to_string(),
                            label: "Size".to_string(),
                            values: vec!["1024x1024".to_string()],
                            default: "1024x1024".to_string(),
                            request_field: Some("size".to_string()),
                            wire_type: MediaParameterWireType::String,
                        },
                        MediaParameterSpec {
                            name: "quality".to_string(),
                            label: "Quality".to_string(),
                            values: vec!["auto".to_string(), "high".to_string()],
                            default: "auto".to_string(),
                            request_field: Some("quality".to_string()),
                            wire_type: MediaParameterWireType::String,
                        },
                        MediaParameterSpec {
                            name: "output_format".to_string(),
                            label: "Output format".to_string(),
                            values: vec!["png".to_string()],
                            default: "png".to_string(),
                            request_field: Some("output_format".to_string()),
                            wire_type: MediaParameterWireType::String,
                        },
                    ],
                }],
            }),
            video: None,
        }
    }

    fn video_media(model_id: &str) -> ProviderMediaDescriptor {
        video_media_with_adapter(MediaExecutionKind::ReplicateVideo, model_id)
    }

    fn video_media_with_adapter(
        adapter: MediaExecutionKind,
        model_id: &str,
    ) -> ProviderMediaDescriptor {
        ProviderMediaDescriptor {
            image: None,
            video: Some(MediaKindDescriptor {
                discovery: None,
                execution: Some(MediaExecutionDescriptor {
                    adapter,
                    base_url: None,
                    path: "/v1/predictions".to_string(),
                    batch: puffer_provider_registry::MediaBatchDescriptor::default(),
                }),
                models: vec![MediaModelDescriptor {
                    id: model_id.to_string(),
                    display_name: Some("Display Video".to_string()),
                    execution: None,
                    operations: vec![MediaOperation::Generate],
                    parameters: vec![
                        MediaParameterSpec {
                            name: "aspect_ratio".to_string(),
                            label: "Aspect ratio".to_string(),
                            values: vec!["16:9".to_string(), "9:16".to_string()],
                            default: "16:9".to_string(),
                            request_field: Some("aspect_ratio".to_string()),
                            wire_type: MediaParameterWireType::String,
                        },
                        MediaParameterSpec {
                            name: "duration_seconds".to_string(),
                            label: "Duration".to_string(),
                            values: vec!["5".to_string(), "8".to_string()],
                            default: "5".to_string(),
                            request_field: Some("duration".to_string()),
                            wire_type: MediaParameterWireType::String,
                        },
                    ],
                }],
            }),
        }
    }

    fn auth_for(provider_id: &str) -> AuthStore {
        let mut auth = AuthStore::default();
        auth.set_api_key(provider_id, "sk-test");
        auth
    }

    #[test]
    fn connected_exact_image_descriptor_appears() {
        let registry = registry_with(vec![provider(
            "openai",
            vec![AuthMode::ApiKey],
            Some(image_media("gpt-image-1")),
        )]);
        let capabilities = resolve_media_capabilities(
            &registry,
            &auth_for("openai"),
            MediaKind::Image,
            MediaOperation::Generate,
            42,
            &MediaDiscoveryCache::default(),
        );

        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].provider_id, "openai");
        assert_eq!(capabilities[0].provider_display_name, "openai");
        assert_eq!(capabilities[0].model_id, "gpt-image-1");
        assert_eq!(capabilities[0].model_display_name, "Display Image");
        assert_eq!(capabilities[0].operation, "generate");
        assert_eq!(capabilities[0].adapter, "images_json");
        assert_eq!(capabilities[0].source, "static");
        assert_eq!(capabilities[0].status, "available");
        assert_eq!(capabilities[0].defaults["size"], "1024x1024");
        assert_eq!(
            capabilities[0].parameters[0],
            MediaCapabilityParameter {
                name: "size".to_string(),
                label: "Size".to_string(),
                values: vec!["1024x1024".to_string()],
                default: "1024x1024".to_string(),
                request_field: Some("size".to_string()),
                wire_type: MediaParameterWireType::String,
            }
        );
    }

    #[test]
    fn connected_provider_with_replicate_video_descriptor_is_available() {
        let registry = registry_with(vec![provider(
            "replicate",
            vec![AuthMode::ApiKey],
            Some(video_media("owner/model-version")),
        )]);
        let auth = auth_for("replicate");

        let capabilities = resolve_media_capabilities(
            &registry,
            &auth,
            MediaKind::Video,
            MediaOperation::Generate,
            42,
            &MediaDiscoveryCache::default(),
        );

        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].adapter, "replicate_video");
        assert_eq!(
            capabilities[0].defaults.get("duration_seconds"),
            Some(&"5".to_string())
        );
    }

    #[test]
    fn relaydance_video_execution_adapter_is_available() {
        assert!(execution_adapter_is_available_for_kind(
            MediaKind::Video,
            MediaExecutionKind::RelaydanceVideo
        ));
    }

    #[test]
    fn video_descriptor_with_image_adapter_is_unavailable_with_adapter_reason() {
        let registry = registry_with(vec![provider(
            "replicate",
            vec![AuthMode::ApiKey],
            Some(video_media_with_adapter(
                MediaExecutionKind::ImagesJson,
                "owner/model-version",
            )),
        )]);
        let auth = auth_for("replicate");

        let capabilities = resolve_media_capabilities(
            &registry,
            &auth,
            MediaKind::Video,
            MediaOperation::Generate,
            42,
            &MediaDiscoveryCache::default(),
        );

        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].provider_id, "replicate");
        assert_eq!(capabilities[0].adapter, "images_json");
        assert_eq!(capabilities[0].status, "unavailable");
        assert_eq!(
            capabilities[0].reason.as_deref(),
            Some("adapter_unavailable")
        );
    }

    #[test]
    fn unauthenticated_video_descriptor_appears_unavailable_with_missing_auth() {
        let registry = registry_with(vec![provider(
            "relaydance",
            vec![AuthMode::ApiKey],
            Some(video_media_with_adapter(
                MediaExecutionKind::RelaydanceVideo,
                "doubao-seedance-2-0-720p",
            )),
        )]);

        let capabilities = resolve_media_capabilities(
            &registry,
            &AuthStore::default(),
            MediaKind::Video,
            MediaOperation::Generate,
            42,
            &MediaDiscoveryCache::default(),
        );

        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].provider_id, "relaydance");
        assert_eq!(capabilities[0].adapter, "relaydance_video");
        assert_eq!(capabilities[0].status, "unavailable");
        assert_eq!(capabilities[0].reason.as_deref(), Some("missing_auth"));
        assert_eq!(capabilities[0].defaults["duration_seconds"], "5");
    }

    #[test]
    fn connected_relaydance_video_descriptor_is_available() {
        let registry = registry_with(vec![provider(
            "relaydance",
            vec![AuthMode::ApiKey],
            Some(video_media_with_adapter(
                MediaExecutionKind::RelaydanceVideo,
                "doubao-seedance-2-0-720p",
            )),
        )]);

        let capabilities = resolve_media_capabilities(
            &registry,
            &auth_for("relaydance"),
            MediaKind::Video,
            MediaOperation::Generate,
            42,
            &MediaDiscoveryCache::default(),
        );

        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].status, "available");
        assert_eq!(capabilities[0].reason, None);
    }

    #[test]
    fn unavailable_video_capability_cannot_validate_generation_selection() {
        let registry = registry_with(vec![provider(
            "relaydance",
            vec![AuthMode::ApiKey],
            Some(video_media_with_adapter(
                MediaExecutionKind::RelaydanceVideo,
                "doubao-seedance-2-0-720p",
            )),
        )]);
        let selected = BTreeMap::new();

        let error = validate_media_generate_selection(
            &registry,
            &AuthStore::default(),
            &MediaGenerationSelection {
                kind: MediaKind::Video,
                provider_id: "relaydance",
                model_id: "doubao-seedance-2-0-720p",
                operation: MediaOperation::Generate,
                adapter: "relaydance_video",
                parameters: &selected,
            },
            42,
            &MediaDiscoveryCache::default(),
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains(
                "selected video model unavailable: relaydance/doubao-seedance-2-0-720p via relaydance_video"
            ),
            "{error}"
        );
    }

    #[test]
    fn providers_without_image_media_or_auth_are_hidden() {
        let registry = registry_with(vec![
            provider("connected-text", vec![AuthMode::ApiKey], None),
            provider(
                "missing-auth",
                vec![AuthMode::ApiKey],
                Some(image_media("gpt-image-1")),
            ),
            provider("auth-free-text", Vec::new(), None),
        ]);
        let capabilities = resolve_media_capabilities(
            &registry,
            &auth_for("connected-text"),
            MediaKind::Image,
            MediaOperation::Generate,
            42,
            &MediaDiscoveryCache::default(),
        );

        assert!(capabilities.is_empty());
    }

    #[test]
    fn auto_models_and_missing_execution_are_hidden() {
        let mut missing_execution = image_media("gpt-image-1");
        missing_execution.image.as_mut().unwrap().execution = None;
        let registry = registry_with(vec![
            provider(
                "auto-provider",
                vec![AuthMode::ApiKey],
                Some(image_media("auto")),
            ),
            provider(
                "no-execution",
                vec![AuthMode::ApiKey],
                Some(missing_execution),
            ),
        ]);
        let mut auth = AuthStore::default();
        auth.set_api_key("auto-provider", "sk-test");
        auth.set_api_key("no-execution", "sk-test");

        let capabilities = resolve_media_capabilities(
            &registry,
            &auth,
            MediaKind::Image,
            MediaOperation::Generate,
            42,
            &MediaDiscoveryCache::default(),
        );

        assert!(capabilities.is_empty());
    }

    #[test]
    fn saved_stale_provider_model_is_rejected() {
        let registry = registry_with(vec![provider(
            "openai",
            vec![AuthMode::ApiKey],
            Some(image_media("gpt-image-1")),
        )]);

        let error = validate_image_generate_selection(
            &registry,
            &auth_for("openai"),
            &ImageGenerationSelection {
                provider_id: "openai",
                model_id: "stale-image",
                adapter: "images_json",
                parameters: &BTreeMap::from([("size".to_string(), "1024x1024".to_string())]),
            },
            42,
            &MediaDiscoveryCache::default(),
        )
        .expect_err("stale model should fail");

        assert_eq!(
            error.to_string(),
            "selected image model unavailable: openai/stale-image via images_json"
        );
    }

    #[test]
    fn unsupported_parameter_value_is_rejected() {
        let registry = registry_with(vec![provider(
            "openai",
            vec![AuthMode::ApiKey],
            Some(image_media("gpt-image-1")),
        )]);

        let error = validate_image_generate_selection(
            &registry,
            &auth_for("openai"),
            &ImageGenerationSelection {
                provider_id: "openai",
                model_id: "gpt-image-1",
                adapter: "images_json",
                parameters: &BTreeMap::from([("size".to_string(), "2048x2048".to_string())]),
            },
            42,
            &MediaDiscoveryCache::default(),
        )
        .expect_err("unsupported size should fail");

        assert_eq!(
            error.to_string(),
            "image generation parameter unsupported: size=2048x2048"
        );
    }

    #[test]
    fn saved_stale_adapter_is_rejected() {
        let registry = registry_with(vec![provider(
            "openai",
            vec![AuthMode::ApiKey],
            Some(image_media("gpt-image-1")),
        )]);

        let error = validate_image_generate_selection(
            &registry,
            &auth_for("openai"),
            &ImageGenerationSelection {
                provider_id: "openai",
                model_id: "gpt-image-1",
                adapter: "stale_adapter",
                parameters: &BTreeMap::new(),
            },
            42,
            &MediaDiscoveryCache::default(),
        )
        .expect_err("stale adapter should fail");

        assert_eq!(
            error.to_string(),
            "selected image model unavailable: openai/gpt-image-1 via stale_adapter"
        );
    }

    #[test]
    fn model_execution_overrides_provider_execution_adapter() {
        let mut media = image_media("image-only-model");
        let image = media.image.as_mut().expect("image media");
        image.execution = Some(MediaExecutionDescriptor {
            adapter: MediaExecutionKind::ChatImageOutput,
            base_url: None,
            path: "/chat/completions".to_string(),
            batch: puffer_provider_registry::MediaBatchDescriptor::default(),
        });
        image.models[0].execution = Some(MediaExecutionDescriptor {
            adapter: MediaExecutionKind::ImagesJson,
            base_url: None,
            path: "/images/generations".to_string(),
            batch: puffer_provider_registry::MediaBatchDescriptor::default(),
        });
        let registry = registry_with(vec![provider(
            "vercel-ai-gateway",
            vec![AuthMode::ApiKey],
            Some(media),
        )]);

        let capabilities = resolve_media_capabilities(
            &registry,
            &auth_for("vercel-ai-gateway"),
            MediaKind::Image,
            MediaOperation::Generate,
            42,
            &MediaDiscoveryCache::default(),
        );

        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].adapter, "images_json");
    }
}
