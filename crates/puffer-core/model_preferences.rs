use puffer_provider_registry::ProviderRegistry;

/// Describes the provider-specific behavior used for model preference commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelPreferenceFamily {
    Anthropic,
    OpenAi,
    Other,
}

/// Returns the provider family that should drive effort and fast-mode behavior.
pub fn provider_preference_family(
    providers: &ProviderRegistry,
    provider_id: &str,
) -> ModelPreferenceFamily {
    let Some(provider) = providers.provider(provider_id) else {
        return infer_provider_family(provider_id, "");
    };
    infer_provider_family(&provider.id, &provider.default_api)
}

/// Returns the slash-command effort levels supported for a provider family.
pub fn supported_effort_levels(family: ModelPreferenceFamily) -> &'static [&'static str] {
    match family {
        ModelPreferenceFamily::Anthropic => &["low", "medium", "high", "max"],
        ModelPreferenceFamily::OpenAi => &["minimal", "low", "medium", "high", "xhigh"],
        ModelPreferenceFamily::Other => &["low", "medium", "high"],
    }
}

/// Returns the default effort level used when a provider switch invalidates the current one.
pub fn default_effort_level(family: ModelPreferenceFamily) -> &'static str {
    match family {
        ModelPreferenceFamily::Anthropic => "high",
        ModelPreferenceFamily::OpenAi => "low",
        ModelPreferenceFamily::Other => "medium",
    }
}

/// Returns whether the provided effort level is accepted for the provider family.
pub fn effort_level_is_supported(family: ModelPreferenceFamily, effort_level: &str) -> bool {
    let normalized = effort_level.trim().to_ascii_lowercase();
    normalized == "auto"
        || normalized == "unset"
        || supported_effort_levels(family)
            .iter()
            .any(|candidate| *candidate == normalized)
}

/// Normalizes an effort level for the given provider family.
pub fn normalized_effort_level(family: ModelPreferenceFamily, effort_level: &str) -> String {
    let normalized = effort_level.trim().to_ascii_lowercase();
    if normalized == "auto" || normalized == "unset" {
        return "auto".to_string();
    }
    if family == ModelPreferenceFamily::OpenAi && normalized == "max" {
        return "high".to_string();
    }
    if family == ModelPreferenceFamily::Anthropic && normalized == "xhigh" {
        return "high".to_string();
    }
    if effort_level_is_supported(family, &normalized) {
        normalized
    } else {
        default_effort_level(family).to_string()
    }
}

fn infer_provider_family(provider_id: &str, default_api: &str) -> ModelPreferenceFamily {
    if provider_id.eq_ignore_ascii_case("anthropic") || default_api.starts_with("anthropic") {
        ModelPreferenceFamily::Anthropic
    } else if provider_id.to_ascii_lowercase().contains("openai")
        || default_api.starts_with("openai")
        || default_api.contains("codex")
    {
        ModelPreferenceFamily::OpenAi
    } else {
        ModelPreferenceFamily::Other
    }
}

#[cfg(test)]
mod tests {
    use super::{
        default_effort_level, effort_level_is_supported, normalized_effort_level,
        ModelPreferenceFamily,
    };

    #[test]
    fn openai_effort_levels_include_xhigh_and_auto() {
        assert!(effort_level_is_supported(
            ModelPreferenceFamily::OpenAi,
            "xhigh"
        ));
        assert!(effort_level_is_supported(
            ModelPreferenceFamily::OpenAi,
            "auto"
        ));
        assert_eq!(
            normalized_effort_level(ModelPreferenceFamily::OpenAi, "max"),
            "high"
        );
        assert_eq!(default_effort_level(ModelPreferenceFamily::OpenAi), "low");
    }

    #[test]
    fn anthropic_effort_levels_reject_openai_only_values() {
        assert!(!effort_level_is_supported(
            ModelPreferenceFamily::Anthropic,
            "xhigh"
        ));
        assert_eq!(
            normalized_effort_level(ModelPreferenceFamily::Anthropic, "xhigh"),
            "high"
        );
    }
}
