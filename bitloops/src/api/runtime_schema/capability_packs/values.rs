use serde_json::Value;

pub(super) fn runtime_options_from_value(value: &Value) -> Vec<String> {
    value
        .get("inference")
        .and_then(|value| value.get("runtimes"))
        .and_then(Value::as_object)
        .map(|items| items.keys().cloned().collect())
        .unwrap_or_default()
}

pub(super) fn profile_options_for_task(value: &Value, task: &str) -> Vec<String> {
    value
        .get("inference")
        .and_then(|value| value.get("profiles"))
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, profile)| {
                    (profile.get("task").and_then(Value::as_str) == Some(task))
                        .then_some(key.clone())
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn string_value_at_path(value: &Value, path: &[&str]) -> Option<String> {
    value_at_path(value, path)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn has_non_empty_value(value: &Value, path: &[&str]) -> bool {
    value_at_path(value, path)
        .map(|value| match value {
            Value::Null => false,
            Value::String(value) => !value.trim().is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Object(items) => !items.is_empty(),
            _ => true,
        })
        .unwrap_or(false)
}

pub(super) fn has_truthy_value(value: &Value, path: &[&str]) -> bool {
    value_at_path(value, path)
        .map(|value| match value {
            Value::Bool(value) => *value,
            Value::Null => false,
            Value::String(value) => !value.trim().is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Object(items) => !items.is_empty(),
            _ => true,
        })
        .unwrap_or(false)
}

pub(super) fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

pub(super) fn read_string_list(value: &Value, path: &[&str]) -> Vec<String> {
    value_at_path(value, path)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
