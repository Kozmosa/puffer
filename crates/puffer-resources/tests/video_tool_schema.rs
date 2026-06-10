#[test]
fn video_generation_tool_schema_accepts_scalar_parameter_values() {
    let tool: serde_json::Value = serde_yaml::from_str(include_str!(
        "../../../resources/internal_tools/video_generation.yaml"
    ))
    .expect("VideoGeneration internal tool YAML");
    let parameter_types = tool["input_schema"]["properties"]["parameters"]["additionalProperties"]
        ["oneOf"]
        .as_array()
        .expect("parameter scalar types");
    let types = parameter_types
        .iter()
        .filter_map(|value| value.get("type").and_then(serde_json::Value::as_str))
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        types,
        std::collections::BTreeSet::from(["boolean", "number", "string"])
    );
}
