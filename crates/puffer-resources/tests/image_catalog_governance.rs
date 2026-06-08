use puffer_provider_registry::{
    MediaBatchMode, MediaExecutionKind, MediaModelDescriptor, MediaOperation, ProviderDescriptor,
};
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

const ALL_PROVIDER_YAMLS: &[(&str, &str)] = &[
    (
        "anthropic",
        include_str!("../../../resources/providers/anthropic.yaml"),
    ),
    (
        "byteplus",
        include_str!("../../../resources/providers/byteplus.yaml"),
    ),
    (
        "cerebras",
        include_str!("../../../resources/providers/cerebras.yaml"),
    ),
    ("groq", include_str!("../../../resources/providers/groq.yaml")),
    (
        "kimi-coding",
        include_str!("../../../resources/providers/kimi-coding.yaml"),
    ),
    (
        "kimi-openai",
        include_str!("../../../resources/providers/kimi-openai.yaml"),
    ),
    (
        "llama-cpp",
        include_str!("../../../resources/providers/llama-cpp.yaml"),
    ),
    (
        "lmstudio",
        include_str!("../../../resources/providers/lmstudio.yaml"),
    ),
    (
        "minicpm5",
        include_str!("../../../resources/providers/minicpm5.yaml"),
    ),
    (
        "minimax",
        include_str!("../../../resources/providers/minimax.yaml"),
    ),
    (
        "minimax-cn",
        include_str!("../../../resources/providers/minimax-cn.yaml"),
    ),
    ("ollama", include_str!("../../../resources/providers/ollama.yaml")),
    (
        "openai",
        include_str!("../../../resources/providers/openai.yaml"),
    ),
    (
        "openrouter",
        include_str!("../../../resources/providers/openrouter.yaml"),
    ),
    (
        "relaydance",
        include_str!("../../../resources/providers/relaydance.yaml"),
    ),
    (
        "vercel-ai-gateway",
        include_str!("../../../resources/providers/vercel-ai-gateway.yaml"),
    ),
    ("vllm", include_str!("../../../resources/providers/vllm.yaml")),
    (
        "worldrouter",
        include_str!("../../../resources/providers/worldrouter.yaml"),
    ),
    ("xai", include_str!("../../../resources/providers/xai.yaml")),
    ("zhipu", include_str!("../../../resources/providers/zhipu.yaml")),
];

fn provider_descriptor(provider_id: &str, yaml: &str) -> ProviderDescriptor {
    let pack: ProviderPack = serde_yaml::from_str(yaml)
        .unwrap_or_else(|err| panic!("{provider_id}.yaml should parse: {err}"));
    assert_eq!(pack.id, provider_id);
    pack.into_descriptor()
}

fn assert_raw_image_executions_declare_batch_mode(provider_id: &str, yaml: &str) {
    let value: serde_yaml::Value =
        serde_yaml::from_str(yaml).unwrap_or_else(|error| panic!("{provider_id}: {error}"));
    let image = value
        .get("media")
        .and_then(|media| media.get("image"))
        .unwrap_or_else(|| panic!("{provider_id}: missing media.image"));
    let execution = image
        .get("execution")
        .unwrap_or_else(|| panic!("{provider_id}: missing media.image.execution"));
    assert!(
        execution
            .get("batch")
            .and_then(|batch| batch.get("mode"))
            .and_then(serde_yaml::Value::as_str)
            .is_some(),
        "{provider_id}: media.image.execution.batch.mode must be explicit"
    );
    if let Some(models) = image.get("models").and_then(serde_yaml::Value::as_sequence) {
        for model in models {
            if let Some(model_execution) = model.get("execution") {
                let model_id = model
                    .get("id")
                    .and_then(serde_yaml::Value::as_str)
                    .unwrap_or("<missing>");
                assert!(
                    model_execution
                        .get("batch")
                        .and_then(|batch| batch.get("mode"))
                        .and_then(serde_yaml::Value::as_str)
                        .is_some(),
                    "{provider_id}/{model_id}: models[].execution.batch.mode must be explicit"
                );
            }
        }
    }
}

fn assert_select_parameter(
    model: &MediaModelDescriptor,
    name: &str,
    label: &str,
    values: &[&str],
    default: &str,
    request_field: &str,
) {
    let parameter = model
        .parameters
        .iter()
        .find(|parameter| parameter.name == name)
        .unwrap_or_else(|| panic!("{} should declare {name}", model.id));

    assert_eq!(parameter.label, label);
    assert_eq!(
        parameter
            .values
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        values
    );
    assert_eq!(parameter.default, default);
    assert_eq!(parameter.request_field.as_deref(), Some(request_field));
}

#[test]
fn bundled_image_executions_declare_explicit_batch_mode() {
    for (provider_id, yaml) in [
        ("openai", include_str!("../../../resources/providers/openai.yaml")),
        ("xai", include_str!("../../../resources/providers/xai.yaml")),
        ("zhipu", include_str!("../../../resources/providers/zhipu.yaml")),
        (
            "byteplus",
            include_str!("../../../resources/providers/byteplus.yaml"),
        ),
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
    ] {
        assert_raw_image_executions_declare_batch_mode(provider_id, yaml);
    }
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
fn relaydance_declares_executable_video_descriptor() {
    let descriptor = provider_descriptor(
        "relaydance",
        include_str!("../../../resources/providers/relaydance.yaml"),
    );
    descriptor
        .validate_media_descriptors()
        .expect("relaydance media descriptor validates");
    let video = descriptor
        .media
        .as_ref()
        .and_then(|media| media.video.as_ref())
        .expect("relaydance video media descriptor");
    let execution = video
        .execution
        .as_ref()
        .expect("relaydance video execution descriptor");

    assert_eq!(execution.adapter, MediaExecutionKind::OpenAiVideo);
    assert_eq!(execution.path, "/v1/video/generations");

    let model = video
        .models
        .iter()
        .find(|model| model.id == "doubao-seedance-2-0-720p")
        .expect("relaydance should include Seedance 2.0 720p");
    assert_eq!(model.display_name.as_deref(), Some("Seedance 2.0 (720p)"));
    assert_eq!(model.operations, vec![MediaOperation::Generate]);

    let parameters = model
        .parameters
        .iter()
        .map(|parameter| {
            (
                parameter.name.as_str(),
                (
                    parameter.default.as_str(),
                    parameter.request_field.as_deref(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(parameters.get("duration"), Some(&("5", Some("seconds"))));
    assert_eq!(
        parameters.get("resolution"),
        Some(&("720p", Some("metadata.resolution")))
    );
    assert_eq!(
        parameters.get("ratio"),
        Some(&("16:9", Some("metadata.ratio")))
    );
}

#[test]
fn only_relaydance_declares_first_pass_video_media() {
    for (provider_id, yaml) in ALL_PROVIDER_YAMLS {
        let descriptor = provider_descriptor(provider_id, yaml);
        let has_video = descriptor
            .media
            .as_ref()
            .and_then(|media| media.video.as_ref())
            .is_some();
        assert_eq!(
            has_video,
            *provider_id == "relaydance",
            "{provider_id} must not declare media.video until a Puffer video adapter exists"
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
fn zhipu_images_json_uses_per_image_batch_mode() {
    let descriptor = provider_descriptor(
        "zhipu",
        include_str!("../../../resources/providers/zhipu.yaml"),
    );
    let image = descriptor
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .expect("zhipu image media descriptor");
    let execution = image
        .execution
        .as_ref()
        .expect("zhipu image execution descriptor");

    assert_eq!(execution.adapter, MediaExecutionKind::ImagesJson);
    assert_eq!(execution.path, "/images/generations");
    assert_eq!(execution.batch.mode, MediaBatchMode::PerImage);
    assert_eq!(execution.batch.max_images_per_call, None);
}

#[test]
fn openai_catalog_declares_current_image_api_models() {
    let descriptor = provider_descriptor(
        "openai",
        include_str!("../../../resources/providers/openai.yaml"),
    );
    let image = descriptor
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .expect("openai image media descriptor");

    assert_eq!(
        image.execution.as_ref().map(|execution| execution.adapter),
        Some(MediaExecutionKind::ImagesJson)
    );
    assert_eq!(
        image
            .execution
            .as_ref()
            .map(|execution| execution.path.as_str()),
        Some("/v1/images/generations")
    );

    let expected = BTreeMap::from([
        ("chatgpt-image-latest", "ChatGPT Image Latest"),
        ("gpt-image-1", "GPT Image 1"),
        ("gpt-image-1-mini", "GPT Image 1 Mini"),
        ("gpt-image-1.5", "GPT Image 1.5"),
        ("gpt-image-2", "GPT Image 2"),
    ]);
    let models_by_id = image
        .models
        .iter()
        .map(|model| (model.id.as_str(), model))
        .collect::<BTreeMap<_, _>>();
    let expected_model_ids = expected.keys().copied().collect::<BTreeSet<_>>();
    let actual_model_ids = models_by_id.keys().copied().collect::<BTreeSet<_>>();

    assert_eq!(
        actual_model_ids, expected_model_ids,
        "OpenAI image catalog should mirror currently callable Image API model ids"
    );
    assert!(
        !actual_model_ids.contains("dall-e-2") && !actual_model_ids.contains("dall-e-3"),
        "OpenAI image catalog should not include DALL-E models removed on 2026-05-12"
    );

    for (model_id, display_name) in &expected {
        let model_id = *model_id;
        let model = models_by_id
            .get(model_id)
            .unwrap_or_else(|| panic!("OpenAI should include {model_id}"));
        assert_eq!(model.display_name.as_deref(), Some(*display_name));
        assert!(
            model.operations.contains(&MediaOperation::Generate),
            "{model_id} should support image generation"
        );
        assert_select_parameter(
            model,
            "quality",
            "Quality",
            &["auto", "low", "medium", "high"],
            "auto",
            "quality",
        );
        assert_select_parameter(
            model,
            "output_format",
            "Output format",
            &["png", "jpeg", "webp"],
            "png",
            "output_format",
        );

        if model_id == "gpt-image-2" {
            assert_select_parameter(
                model,
                "size",
                "Size",
                &[
                    "auto",
                    "1024x1024",
                    "1024x1536",
                    "1536x1024",
                    "2048x2048",
                    "2048x1152",
                    "2560x1440",
                    "3840x2160",
                    "2160x3840",
                ],
                "auto",
                "size",
            );
        } else {
            assert_select_parameter(
                model,
                "size",
                "Size",
                &["auto", "1024x1024", "1024x1536", "1536x1024"],
                "auto",
                "size",
            );
        }
    }
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
        ("seedream-4-0-250828", "Seedream 4.0"),
    ]);
    let models_by_id = image
        .models
        .iter()
        .map(|model| (model.id.as_str(), model))
        .collect::<BTreeMap<_, _>>();
    let expected_model_ids = expected.keys().copied().collect::<BTreeSet<_>>();
    let actual_model_ids = models_by_id.keys().copied().collect::<BTreeSet<_>>();

    assert_eq!(
        actual_model_ids, expected_model_ids,
        "BytePlus image catalog should exactly match the current native Seedream allowlist"
    );

    for (model_id, display_name) in &expected {
        let model_id = *model_id;
        let display_name = *display_name;
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
        assert_select_parameter(model, "size", "Size", &["2K"], "2K", "size");
        assert_select_parameter(
            model,
            "response_format",
            "Response format",
            &["b64_json", "url"],
            "b64_json",
            "response_format",
        );
        assert_select_parameter(
            model,
            "sequential_image_generation",
            "Sequential image generation",
            &["disabled", "auto"],
            "disabled",
            "sequential_image_generation",
        );
        if model_id == "seedream-5-0-260128" {
            assert_eq!(
                parameter_names,
                BTreeSet::from([
                    "size",
                    "output_format",
                    "response_format",
                    "sequential_image_generation",
                ]),
                "{model_id} should declare exactly the adapter-supported BytePlus parameters"
            );
            assert_select_parameter(
                model,
                "output_format",
                "Output format",
                &["png", "jpeg"],
                "jpeg",
                "output_format",
            );
        } else {
            assert_eq!(
                parameter_names,
                BTreeSet::from(["size", "response_format", "sequential_image_generation"]),
                "{model_id} should omit unsupported output_format but keep API-level parameters"
            );
        }
    }
}
