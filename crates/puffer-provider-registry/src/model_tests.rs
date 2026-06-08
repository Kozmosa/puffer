use super::*;

fn provider_with_media_yaml(media_yaml: &str) -> String {
    format!(
        r#"
id: test-provider
display_name: Test Provider
base_url: https://api.test-provider.example
default_api: openai-responses
auth_modes:
  - api_key
models: []
{media_yaml}
"#
    )
}

fn provider_with_basic_image_execution(execution_yaml: &str) -> String {
    let media_yaml = format!(
        r#"
media:
  image:
    execution:
{execution_yaml}
    models:
      - id: gpt-image-1
        operations:
          - generate
"#
    );
    provider_with_media_yaml(&media_yaml)
}

#[test]
fn existing_provider_yaml_parses_without_media() {
    let yaml = include_str!("../../../resources/providers/anthropic.yaml");
    let provider: ProviderDescriptor = serde_yaml::from_str(yaml).expect("anthropic yaml parses");

    assert_eq!(provider.id, "anthropic");
    assert!(provider.media.is_none());
    provider
        .validate_media_descriptors()
        .expect("missing media is valid");
}

#[test]
fn valid_image_media_descriptor_parses_and_validates() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    discovery:
      adapter: static
    execution:
      adapter: images_json
      base_url: https://api.test-provider.example
      path: /v1/images/generations
      batch:
        mode: exact
        max_images_per_call: 4
    models:
      - id: gpt-image-1
        display_name: GPT Image 1
        operations:
          - generate
        parameters:
          - name: size
            label: Size
            values:
              - 1024x1024
              - 1536x1024
            default: 1024x1024
            request_field: size
          - name: quality
            label: Quality
            values:
              - auto
              - high
            default: auto
            request_field: quality
          - name: output_format
            label: Output format
            values:
              - png
              - jpeg
            default: png
            request_field: output_format
"#,
    );

    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");

    provider
        .validate_media_descriptors()
        .expect("media descriptor validates");
    let image = provider
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .expect("image media");
    assert_eq!(
        image.execution.as_ref().map(|execution| execution.adapter),
        Some(MediaExecutionKind::ImagesJson)
    );
    assert_eq!(
        image
            .execution
            .as_ref()
            .and_then(|execution| execution.base_url.as_deref()),
        Some("https://api.test-provider.example")
    );
    let execution = image.execution.as_ref().expect("image execution");
    assert_eq!(execution.batch.mode, MediaBatchMode::Exact);
    assert_eq!(execution.batch.max_images_per_call, Some(4));
    assert_eq!(image.models[0].operations, vec![MediaOperation::Generate]);
}

#[test]
fn provider_media_descriptor_accepts_video_models() {
    let yaml = r#"
id: replicate
display_name: Replicate
base_url: https://api.replicate.com
default_api: openai-responses
auth_modes: [api_key]
media:
  video:
    execution:
      adapter: replicate_video
      path: /v1/predictions
    models:
      - id: owner/model-version
        display_name: Video Model
        operations: [generate]
        parameters:
          - name: aspect_ratio
            label: Aspect ratio
            values: ["16:9", "9:16"]
            default: "16:9"
          - name: duration
            label: Duration
            values: ["5", "8"]
            default: "5"
"#;

    let provider: ProviderDescriptor = serde_yaml::from_str(yaml).expect("parse provider");
    provider
        .validate_media_descriptors()
        .expect("valid media descriptor");
    assert!(provider.media.unwrap().video.is_some());
}

#[test]
fn provider_media_descriptor_rejects_invalid_video_parameter_default() {
    let yaml = r#"
id: replicate
display_name: Replicate
base_url: https://api.replicate.com
default_api: openai-responses
auth_modes: [api_key]
media:
  video:
    execution:
      adapter: replicate_video
      path: /v1/predictions
    models:
      - id: owner/model-version
        operations: [generate]
        parameters:
          - name: duration
            label: Duration
            values: ["5"]
            default: "8"
"#;

    let provider: ProviderDescriptor = serde_yaml::from_str(yaml).expect("parse provider");
    let error = provider
        .validate_media_descriptors()
        .unwrap_err()
        .to_string();
    assert!(error.contains("media.video.models[0].parameters[0].default"));
}

#[test]
fn missing_image_execution_batch_defaults_to_per_image() {
    let yaml = provider_with_basic_image_execution(
        "      adapter: images_json\n      path: /v1/images/generations",
    );
    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");
    let execution = provider
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .and_then(|image| image.execution.as_ref())
        .expect("image execution");

    assert_eq!(execution.batch.mode, MediaBatchMode::PerImage);
    assert_eq!(execution.batch.max_images_per_call, None);
    provider
        .validate_media_descriptors()
        .expect("default per-image batch validates");
}

#[test]
fn per_image_batch_rejects_max_images_per_call() {
    let yaml = provider_with_basic_image_execution(
        "      adapter: images_json\n      path: /v1/images/generations\n      batch:\n        mode: per_image\n        max_images_per_call: 1",
    );
    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");

    let error = provider
        .validate_media_descriptors()
        .expect_err("per-image mode must not carry an exact batch limit");

    assert!(
        error
            .to_string()
            .contains("media.image.execution.batch.max_images_per_call"),
        "{error}"
    );
}

#[test]
fn exact_batch_requires_at_least_two_images_per_call() {
    let yaml = provider_with_basic_image_execution(
        "      adapter: images_json\n      path: /v1/images/generations\n      batch:\n        mode: exact\n        max_images_per_call: 1",
    );
    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");

    let error = provider
        .validate_media_descriptors()
        .expect_err("exact mode needs a real batch size");

    assert!(
        error
            .to_string()
            .contains("media.image.execution.batch.max_images_per_call"),
        "{error}"
    );
}

#[test]
fn auto_image_model_is_rejected_by_validation() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    execution:
      adapter: images_json
      path: /v1/images/generations
    models:
      - id: auto
        operations:
          - generate
"#,
    );
    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");

    let error = provider
        .validate_media_descriptors()
        .expect_err("auto model is invalid");

    assert!(
        error.to_string().contains("media.image.models[0].id"),
        "{error}"
    );
}

#[test]
fn missing_image_execution_parses_and_validates_for_later_availability_skip() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    models:
      - id: gpt-image-1
        operations:
          - generate
"#,
    );
    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");

    assert!(provider
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .and_then(|image| image.execution.as_ref())
        .is_none());
    provider
        .validate_media_descriptors()
        .expect("missing execution only affects availability");
}

#[test]
fn empty_image_execution_path_is_rejected_by_validation() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    execution:
      adapter: images_json
      path: ""
    models:
      - id: gpt-image-1
        operations:
          - generate
"#,
    );
    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");

    let error = provider
        .validate_media_descriptors()
        .expect_err("empty execution path is invalid");

    assert!(error.to_string().contains("execution.path"), "{error}");
}

#[test]
fn old_top_level_image_batch_limit_is_rejected() {
    let yaml = provider_with_basic_image_execution(
        "      adapter: images_json\n      path: /v1/images/generations\n      max_images_per_call: 4",
    );
    let error = serde_yaml::from_str::<ProviderDescriptor>(&yaml)
        .expect_err("old top-level batch limit should be rejected");

    assert!(error.to_string().contains("max_images_per_call"), "{error}");
}

#[test]
fn empty_declared_image_parameter_array_is_rejected_by_validation() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    execution:
      adapter: images_json
      path: /v1/images/generations
    models:
      - id: gpt-image-1
        operations:
          - generate
        parameters:
          - name: size
            label: Size
            values: []
            default: 1024x1024
"#,
    );
    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");

    let error = provider
        .validate_media_descriptors()
        .expect_err("empty declared parameter list is invalid");

    assert!(
        error.to_string().contains("parameters[0].values"),
        "{error}"
    );
}

#[test]
fn image_media_descriptor_uses_select_parameter_specs() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    execution:
      adapter: images_json
      path: /v1/images/generations
    models:
      - id: gpt-image-1
        display_name: GPT Image 1
        operations:
          - generate
        parameters:
          - name: size
            label: Size
            values:
              - 1024x1024
              - 1536x1024
            default: 1024x1024
            request_field: size
          - name: output_format
            label: Output format
            values:
              - png
              - jpeg
            default: png
"#,
    );

    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");

    provider
        .validate_media_descriptors()
        .expect("media descriptor validates");
    let image = provider
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .expect("image media");
    assert_eq!(
        image.execution.as_ref().map(|execution| execution.adapter),
        Some(MediaExecutionKind::ImagesJson)
    );
    assert_eq!(image.models[0].parameters[0].name, "size");
    assert_eq!(image.models[0].parameters[0].default, "1024x1024");
    assert_eq!(
        image.models[0].parameters[0].request_field.as_deref(),
        Some("size")
    );
}

#[test]
fn image_model_can_override_provider_execution_adapter() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    execution:
      adapter: chat_image_output
      path: /chat/completions
    models:
      - id: image-only-model
        operations:
          - generate
        execution:
          adapter: images_json
          path: /images/generations
"#,
    );

    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");

    provider
        .validate_media_descriptors()
        .expect("media descriptor validates");
    let model = provider
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .and_then(|image| image.models.first())
        .expect("image model");
    assert_eq!(
        model.execution.as_ref().map(|execution| execution.adapter),
        Some(MediaExecutionKind::ImagesJson)
    );
}

#[test]
fn media_execution_kind_parses_relaydance_video() {
    let kind: MediaExecutionKind = serde_yaml::from_str("relaydance_video").expect("parse");
    assert_eq!(kind, MediaExecutionKind::RelaydanceVideo);
}

#[test]
fn media_execution_kind_parses_byteplus_video() {
    let kind: MediaExecutionKind = serde_yaml::from_str("byteplus_video").expect("parse");
    assert_eq!(kind, MediaExecutionKind::BytePlusVideo);
}

#[test]
fn media_execution_kind_rejects_openai_video() {
    let error = serde_yaml::from_str::<MediaExecutionKind>("openai_video").unwrap_err();
    assert!(error.to_string().contains("unknown variant"));
}

#[test]
fn non_select_image_parameter_kind_is_rejected() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    execution:
      adapter: images_json
      path: /v1/images/generations
    models:
      - id: gpt-image-1
        operations:
          - generate
        parameters:
          - name: width
            kind: number
            values:
              - "1024"
            default: "1024"
"#,
    );

    let error = serde_yaml::from_str::<ProviderDescriptor>(&yaml)
        .expect_err("non-select parameter kind should be rejected");

    assert!(error.to_string().contains("kind"), "{error}");
}
