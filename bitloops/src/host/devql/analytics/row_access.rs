use anyhow::Result;
use serde_json::Value;

pub(super) fn row_string(row: &Value, key: &str) -> String {
    match row.get(key) {
        Some(Value::Null) | None => String::new(),
        Some(Value::String(value)) => value.trim().to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

pub(super) fn optional_row_string(row: &Value, key: &str) -> Option<String> {
    let value = row_string(row, key);
    (!value.is_empty()).then_some(value)
}

pub(super) fn row_i64(row: &Value, key: &str) -> i64 {
    match row.get(key) {
        Some(Value::Number(number)) => number.as_i64().unwrap_or_default(),
        Some(Value::String(text)) => text.trim().parse::<i64>().unwrap_or_default(),
        Some(Value::Bool(value)) => i64::from(*value),
        _ => 0,
    }
}

pub(super) fn set_row_string(row: &mut Value, key: &str, value: String) {
    if let Some(object) = row.as_object_mut() {
        object.insert(key.to_string(), Value::String(value));
    }
}

pub(super) fn sql_in_list(values: &[String], escape: fn(&str) -> String) -> String {
    if values.is_empty() {
        return "''".to_string();
    }
    values
        .iter()
        .map(|value| format!("'{}'", escape(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn is_missing_table_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("no such table")
        || message.contains("does not exist")
        || message.contains("catalog error")
}

pub(super) fn ignore_missing_table(error: anyhow::Error) -> Result<Vec<Value>> {
    if is_missing_table_error(&error) {
        Ok(Vec::new())
    } else {
        Err(error)
    }
}
