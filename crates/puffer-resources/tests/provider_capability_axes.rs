use puffer_provider_registry::{AxisRole, ControlKind, Variants};

fn provider(file: &str) -> puffer_provider_registry::ProviderDescriptor {
    // `cargo test` sets CWD to the package root; the provider YAMLs live at the
    // repo root, so resolve relative to CARGO_MANIFEST_DIR (crates/puffer-resources).
    let path = format!(
        "{}/../../resources/providers/{file}.yaml",
        env!("CARGO_MANIFEST_DIR")
    );
    let yaml = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {file}"));
    serde_yaml::from_str(&yaml).unwrap_or_else(|e| panic!("parse {file}: {e}"))
}

#[test]
fn byteplus_seedance_declares_param_axes_and_single_variant() {
    let p = provider("byteplus");
    let video = p.media.as_ref().unwrap().video.as_ref().unwrap();
    let m = video
        .models
        .iter()
        .find(|m| m.id == "dreamina-seedance-2-0")
        .expect("model");
    let res = m
        .axes
        .iter()
        .find(|a| a.id == "resolution")
        .expect("resolution");
    assert_eq!(res.role, AxisRole::Param);
    assert!(
        matches!(&res.control, ControlKind::Enum { values, .. } if values.contains(&"1080p".to_string()))
    );
    assert!(matches!(m.variants, Variants::Single(_)));
}
