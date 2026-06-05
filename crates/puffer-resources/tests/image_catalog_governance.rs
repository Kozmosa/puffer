use puffer_provider_registry::{MediaExecutionKind, MediaOperation, ProviderDescriptor};
use puffer_resources::ProviderPack;
use std::collections::{BTreeMap, BTreeSet};

const IMAGE_PROVIDER_YAMLS: &[(&str, &str)] = &[
    (
        "byteplus",
        include_str!("../../../resources/providers/byteplus.yaml"),
    ),
    (
        "openai",
        include_str!("../../../resources/providers/openai.yaml"),
    ),
    (
        "zhipu",
        include_str!("../../../resources/providers/zhipu.yaml"),
    ),
    ("xai", include_str!("../../../resources/providers/xai.yaml")),
    (
        "minimax",
        include_str!("../../../resources/providers/minimax.yaml"),
    ),
    (
        "minimax-cn",
        include_str!("../../../resources/providers/minimax-cn.yaml"),
    ),
    (
        "openrouter",
        include_str!("../../../resources/providers/openrouter.yaml"),
    ),
    (
        "vercel-ai-gateway",
        include_str!("../../../resources/providers/vercel-ai-gateway.yaml"),
    ),
];

fn provider_descriptor(provider_id: &str, yaml: &str) -> ProviderDescriptor {
    let pack: ProviderPack = serde_yaml::from_str(yaml)
        .unwrap_or_else(|err| panic!("{provider_id}.yaml should parse: {err}"));
    assert_eq!(pack.id, provider_id);
    pack.into_descriptor()
}

#[test]
fn bundled_image_provider_descriptors_validate_and_do_not_duplicate_model_ids() {
    for &(provider_id, yaml) in IMAGE_PROVIDER_YAMLS {
        let descriptor = provider_descriptor(provider_id, yaml);
        descriptor
            .validate_media_descriptors()
            .unwrap_or_else(|err| panic!("{provider_id} media descriptor validates: {err}"));
        let Some(image) = descriptor
            .media
            .as_ref()
            .and_then(|media| media.image.as_ref())
        else {
            continue;
        };
        let ids = image
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>();
        let unique_ids = ids.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(
            ids.len(),
            unique_ids.len(),
            "{provider_id} image model ids must be unique"
        );
    }
}

#[test]
fn native_image_providers_do_not_use_gateway_alias_model_ids() {
    let native_provider_ids = BTreeSet::from([
        "byteplus",
        "openai",
        "zhipu",
        "xai",
        "minimax",
        "minimax-cn",
    ]);

    for &(provider_id, yaml) in IMAGE_PROVIDER_YAMLS {
        if !native_provider_ids.contains(provider_id) {
            continue;
        }
        let descriptor = provider_descriptor(provider_id, yaml);
        let image = descriptor
            .media
            .as_ref()
            .and_then(|media| media.image.as_ref())
            .unwrap_or_else(|| panic!("{provider_id} should declare image media"));
        for model in &image.models {
            assert!(
                !model.id.contains('/'),
                "{provider_id} native image model {} must not be a gateway alias",
                model.id
            );
        }
    }
}

#[test]
fn vercel_static_image_models_have_images_json_execution_overrides() {
    let descriptor = provider_descriptor(
        "vercel-ai-gateway",
        include_str!("../../../resources/providers/vercel-ai-gateway.yaml"),
    );
    let image = descriptor
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .expect("vercel image media descriptor");

    assert_eq!(
        image.execution.as_ref().map(|execution| execution.adapter),
        Some(MediaExecutionKind::ChatImageOutput),
        "provider-level Vercel image execution should remain chat image output"
    );

    for model in &image.models {
        let execution = model
            .execution
            .as_ref()
            .unwrap_or_else(|| panic!("{} must override execution", model.id));
        assert_eq!(
            execution.adapter,
            MediaExecutionKind::ImagesJson,
            "{} must execute through Images JSON",
            model.id
        );
        assert_eq!(
            execution.path, "/images/generations",
            "{} must use the Vercel Images JSON path",
            model.id
        );
    }
}

#[test]
fn openrouter_remains_discovery_driven_without_static_image_fallbacks() {
    let descriptor = provider_descriptor(
        "openrouter",
        include_str!("../../../resources/providers/openrouter.yaml"),
    );
    let image = descriptor
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .expect("openrouter image media descriptor");
    assert!(
        image.models.is_empty(),
        "OpenRouter should not add static image fallback models"
    );
}

#[test]
fn byteplus_catalog_declares_only_current_native_seedream_models() {
    let descriptor = provider_descriptor(
        "byteplus",
        include_str!("../../../resources/providers/byteplus.yaml"),
    );
    let image = descriptor
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .expect("byteplus image media descriptor");

    assert_eq!(
        image.execution.as_ref().map(|execution| execution.adapter),
        Some(MediaExecutionKind::ImagesJson)
    );
    assert_eq!(
        image
            .execution
            .as_ref()
            .map(|execution| execution.path.as_str()),
        Some("/images/generations")
    );

    let expected = BTreeMap::from([
        ("seedream-5-0-260128", "Seedream 5.0 Lite"),
        ("seedream-4-5-251128", "Seedream 4.5"),
    ]);
    let models_by_id = image
        .models
        .iter()
        .map(|model| (model.id.as_str(), model))
        .collect::<BTreeMap<_, _>>();

    for (model_id, display_name) in expected {
        let model = models_by_id
            .get(model_id)
            .unwrap_or_else(|| panic!("BytePlus should include {model_id}"));
        assert_eq!(model.display_name.as_deref(), Some(display_name));
        assert!(
            model.operations.contains(&MediaOperation::Generate),
            "{model_id} should support image generation"
        );
        let parameter_names = model
            .parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<BTreeSet<_>>();
        assert!(
            parameter_names.is_subset(&BTreeSet::from(["size", "output_format"])),
            "{model_id} should only declare adapter-supported BytePlus parameters"
        );
    }

    for forbidden in [
        "seedream-5-0-lite-260128",
        "dola-seedream-5-0-lite-260128",
        "seedream-4-0-250828",
        "seedream-3-0-t2i-250415",
        "bytedance/seedream-4.0",
        "bytedance/seedream-4.5",
        "bytedance/seedream-5.0-lite",
    ] {
        assert!(
            !models_by_id.contains_key(forbidden),
            "BytePlus should not include {forbidden}"
        );
    }
}
