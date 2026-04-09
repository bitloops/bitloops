use serde_json::Value;

pub(super) fn parse_clone_json_string_array(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(raw)) => serde_json::from_str::<Vec<String>>(raw).unwrap_or_default(),
        _ => Vec::new(),
    }
}

pub(super) fn parse_json_f32_array(value: Option<&Value>) -> Vec<f32> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_f64)
            .map(|value| value as f32)
            .filter(|value| value.is_finite())
            .collect(),
        Some(Value::String(raw)) => serde_json::from_str::<Vec<f32>>(raw)
            .unwrap_or_default()
            .into_iter()
            .filter(|value| value.is_finite())
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn value_as_usize(value: &Value) -> Option<usize> {
    if let Some(value) = value.as_u64() {
        return usize::try_from(value).ok();
    }
    if let Some(value) = value.as_i64() {
        return usize::try_from(value).ok();
    }
    value.as_str()?.trim().parse::<usize>().ok()
}
