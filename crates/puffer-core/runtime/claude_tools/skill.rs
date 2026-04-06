use anyhow::{anyhow, bail, Result};
use puffer_resources::LoadedResources;
use serde::Deserialize;
use serde_json::Value;
use std::fmt::Write as _;

const COMMAND_NAME_TAG: &str = "command-name";

#[derive(Debug, Deserialize)]
struct SkillToolInput {
    skill: String,
    #[serde(default)]
    args: Option<String>,
}

fn normalize_skill_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("Skill name cannot be empty");
    }
    let without_slash = trimmed.strip_prefix('/').unwrap_or(trimmed);
    if without_slash.trim().is_empty() {
        bail!("Skill name cannot be empty");
    }
    Ok(without_slash.to_string())
}

/// Executes Claude-style `Skill` tool input by resolving one loaded skill and
/// returning an inline command payload suitable for model consumption.
pub fn execute_claude_skill_tool(resources: &LoadedResources, input: Value) -> Result<String> {
    let parsed: SkillToolInput = serde_json::from_value(input)?;
    let normalized_skill = normalize_skill_name(&parsed.skill)?;
    let skill = resources
        .skills
        .iter()
        .find(|item| item.value.name == normalized_skill)
        .or_else(|| {
            resources
                .skills
                .iter()
                .find(|item| item.value.name.eq_ignore_ascii_case(&normalized_skill))
        })
        .ok_or_else(|| anyhow!("unknown skill `{normalized_skill}`"))?;

    if skill.value.disable_model_invocation {
        bail!(
            "skill `{}` cannot be used with Skill tool due to disable-model-invocation",
            skill.value.name
        );
    }

    let mut output = String::new();
    let _ = writeln!(
        &mut output,
        "<{COMMAND_NAME_TAG}>{}</{COMMAND_NAME_TAG}>",
        skill.value.name
    );
    let _ = writeln!(&mut output, "Skill {}", skill.value.name);
    let _ = writeln!(&mut output, "{}", skill.value.description);
    if let Some(args) = parsed.args.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        let _ = writeln!(&mut output, "args: {args}");
    }
    let _ = writeln!(
        &mut output,
        "\n<skill name=\"{}\">\n{}\n</skill>",
        skill.value.name,
        skill.value.content.trim()
    );
    Ok(output.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_resources::{LoadedItem, SkillSpec, SourceInfo, SourceKind};
    use serde_json::json;

    fn sample_resources() -> LoadedResources {
        LoadedResources {
            skills: vec![
                LoadedItem {
                    value: SkillSpec {
                        name: "review-pr".to_string(),
                        description: "Review one pull request".to_string(),
                        content: "Review prompt body".to_string(),
                        disable_model_invocation: false,
                    },
                    source_info: SourceInfo {
                        path: "skills/review-pr/SKILL.md".into(),
                        kind: SourceKind::Builtin,
                    },
                },
                LoadedItem {
                    value: SkillSpec {
                        name: "hidden".to_string(),
                        description: "Hidden skill".to_string(),
                        content: "Top secret".to_string(),
                        disable_model_invocation: true,
                    },
                    source_info: SourceInfo {
                        path: "skills/hidden/SKILL.md".into(),
                        kind: SourceKind::Builtin,
                    },
                },
            ],
            ..LoadedResources::default()
        }
    }

    #[test]
    fn executes_skill_with_command_tag_and_args() {
        let output = execute_claude_skill_tool(
            &sample_resources(),
            json!({"skill": "/review-pr", "args": "123"}),
        )
        .unwrap();
        assert!(output.contains("<command-name>review-pr</command-name>"));
        assert!(output.contains("<skill name=\"review-pr\">"));
        assert!(output.contains("args: 123"));
    }

    #[test]
    fn rejects_unknown_skill() {
        let error = execute_claude_skill_tool(&sample_resources(), json!({"skill": "does-not-exist"}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown skill"));
    }

    #[test]
    fn rejects_skill_with_disabled_model_invocation() {
        let error = execute_claude_skill_tool(&sample_resources(), json!({"skill": "hidden"}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("disable-model-invocation"));
    }
}
