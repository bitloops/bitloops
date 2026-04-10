use super::*;

// SQL dialect helpers, JSON serialization utilities, and timestamp expressions.

pub(super) fn sql_nullable_text(value: Option<&str>) -> String {
    value
        .map(|text| format!("'{}'", esc_pg(text)))
        .unwrap_or_else(|| "NULL".to_string())
}

pub(super) fn sql_json_text_array(relational: &RelationalStorage, values: &[String]) -> String {
    let raw = esc_pg(&serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string()));
    match relational.dialect() {
        RelationalDialect::Postgres => format!("'{raw}'::jsonb"),
        RelationalDialect::Sqlite => format!("'{raw}'"),
    }
}

#[cfg(test)]
pub(super) fn sql_jsonb_text_array(values: &[String]) -> String {
    format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string()))
    )
}

pub(crate) fn sql_json_value(relational: &RelationalStorage, value: &Value) -> String {
    let raw = esc_pg(&value.to_string());
    match relational.dialect() {
        RelationalDialect::Postgres => format!("'{raw}'::jsonb"),
        RelationalDialect::Sqlite => format!("'{raw}'"),
    }
}

pub(crate) fn sql_now(relational: &RelationalStorage) -> &'static str {
    match relational.dialect() {
        RelationalDialect::Postgres => "now()",
        RelationalDialect::Sqlite => "datetime('now')",
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn parse_json_array_strings(value: Option<&Value>) -> Vec<String> {
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

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn parse_json_value_or_default(value: Option<&Value>, default: Value) -> Value {
    match value {
        Some(Value::String(raw)) => serde_json::from_str(raw).unwrap_or(default),
        Some(other) => other.clone(),
        None => default,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn parse_nullable_i32(value: Option<&Value>) -> Option<i32> {
    value.and_then(|value| {
        value
            .as_i64()
            .and_then(|raw| i32::try_from(raw).ok())
            .or_else(|| value.as_str().and_then(|raw| raw.parse::<i32>().ok()))
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn parse_required_i32(value: Option<&Value>) -> i32 {
    parse_nullable_i32(value).unwrap_or_default()
}
