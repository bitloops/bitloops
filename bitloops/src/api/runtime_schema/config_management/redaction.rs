use serde_json::Value;

use super::types::REDACTED_VALUE;

pub(super) fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

pub(super) fn redact_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    if is_secret_segment(key) {
                        (key.clone(), Value::String(REDACTED_VALUE.to_string()))
                    } else {
                        (key.clone(), redact_json_value(value))
                    }
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_json_value).collect()),
        _ => value.clone(),
    }
}

pub(super) fn is_secret_path(path: &[&str]) -> bool {
    path.iter().any(|segment| is_secret_segment(segment))
}

pub(super) fn is_secret_path_segments(path: &[String]) -> bool {
    path.iter().any(|segment| is_secret_segment(segment))
}

fn is_secret_segment(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    lower.contains("token")
        || lower.contains("secret")
        || lower.contains("password")
        || lower.contains("api_key")
        || lower.contains("credentials")
}
