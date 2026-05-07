use serde_json::Value;

/// Returns a compact header summary for an `AskUserQuestion` tool input.
pub(super) fn input_summary(input: Option<&Value>) -> Option<String> {
    let questions = input?.get("questions")?.as_array()?;
    let first = questions
        .first()?
        .get("question")
        .and_then(Value::as_str)
        .map(normalize_inline_text)
        .filter(|question| !question.is_empty())?;
    if questions.len() > 1 {
        Some(format!("{first} (+{} more)", questions.len() - 1))
    } else {
        Some(first)
    }
}

/// Returns selected-answer display lines for an `AskUserQuestion` tool result.
pub(super) fn output_lines(output: &str) -> Option<Vec<String>> {
    let parsed = serde_json::from_str::<Value>(output).ok()?;
    let answers = parsed.get("answers").and_then(Value::as_object)?;
    if answers.is_empty() {
        let pending = parsed
            .get("pending")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let line = if pending {
            "Awaiting answer"
        } else {
            "No answer selected"
        };
        return Some(vec![line.to_string()]);
    }

    let mut lines = Vec::new();
    let mut used_questions = Vec::new();
    if let Some(questions) = parsed.get("questions").and_then(Value::as_array) {
        for question in questions {
            let Some(question_text) = question.get("question").and_then(Value::as_str) else {
                continue;
            };
            let Some(answer) = answers.get(question_text) else {
                continue;
            };
            lines.push(answer_line(question_text, answer));
            used_questions.push(question_text.to_string());
        }
    }

    for (question_text, answer) in answers {
        if used_questions
            .iter()
            .any(|used_question| used_question == question_text)
        {
            continue;
        }
        lines.push(answer_line(question_text, answer));
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

fn answer_line(question: &str, answer: &Value) -> String {
    format!(
        "{}: {}",
        normalize_inline_text(question),
        answer_value_text(answer)
    )
}

fn answer_value_text(value: &Value) -> String {
    match value {
        Value::String(text) => {
            let normalized = normalize_inline_text(text);
            if normalized.is_empty() {
                "(empty)".to_string()
            } else {
                normalized
            }
        }
        Value::Array(items) => {
            let rendered = items.iter().map(answer_value_text).collect::<Vec<_>>();
            if rendered.is_empty() {
                "(none)".to_string()
            } else {
                rendered.join(", ")
            }
        }
        Value::Null => "(none)".to_string(),
        Value::Bool(_) | Value::Number(_) => value.to_string(),
        Value::Object(_) => serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()),
    }
}

fn normalize_inline_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
