pub(super) fn row_text<'a>(row: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(super) fn row_i64(row: &serde_json::Value, key: &str) -> Option<i64> {
    row.get(key).and_then(|value| match value {
        serde_json::Value::Number(number) => number.as_i64(),
        serde_json::Value::String(raw) => raw.parse::<i64>().ok(),
        _ => None,
    })
}

pub(super) fn sql_nullable_text(value: Option<&str>) -> String {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => format!("'{}'", crate::host::devql::esc_pg(value)),
        None => "NULL".to_string(),
    }
}

pub(super) fn sql_nullable_i64(value: Option<i64>) -> String {
    value
        .map(|number| number.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

pub(super) fn sql_nullable_timestamptz(value: Option<&str>) -> String {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => format!(
            "NULLIF('{}', '')::timestamptz",
            crate::host::devql::esc_pg(value)
        ),
        None => "NULL".to_string(),
    }
}

pub(super) fn sql_jsonb_text(value: Option<&serde_json::Value>, default_json: &str) -> String {
    let json_text = match value {
        Some(serde_json::Value::Null) | None => default_json.to_string(),
        Some(serde_json::Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                default_json.to_string()
            } else if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
                trimmed.to_string()
            } else {
                default_json.to_string()
            }
        }
        Some(other) => other.to_string(),
    };
    format!("'{}'::jsonb", crate::host::devql::esc_pg(&json_text))
}
