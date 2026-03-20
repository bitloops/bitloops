// SQL dialect helpers, JSON serialization utilities, and timestamp expressions.

fn sql_nullable_text(value: Option<&str>) -> String {
    value
        .map(|text| format!("'{}'", esc_pg(text)))
        .unwrap_or_else(|| "NULL".to_string())
}

fn sql_json_text_array(relational: &RelationalStorage, values: &[String]) -> String {
    let raw = esc_pg(&serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string()));
    match relational.dialect() {
        RelationalDialect::Postgres => format!("'{raw}'::jsonb"),
        RelationalDialect::Sqlite => format!("'{raw}'"),
    }
}

#[cfg(test)]
fn sql_jsonb_text_array(values: &[String]) -> String {
    format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string()))
    )
}

fn sql_json_value(relational: &RelationalStorage, value: &Value) -> String {
    let raw = esc_pg(&value.to_string());
    match relational.dialect() {
        RelationalDialect::Postgres => format!("'{raw}'::jsonb"),
        RelationalDialect::Sqlite => format!("'{raw}'"),
    }
}

fn sql_now(relational: &RelationalStorage) -> &'static str {
    match relational.dialect() {
        RelationalDialect::Postgres => "now()",
        RelationalDialect::Sqlite => "datetime('now')",
    }
}

fn is_supported_symbol_language(language: &str) -> bool {
    resolve_language_pack_owner(language).is_some()
}

fn updated_at_unix_expr(relational: &RelationalStorage) -> &'static str {
    match relational.dialect() {
        RelationalDialect::Postgres => "EXTRACT(EPOCH FROM updated_at)::BIGINT",
        RelationalDialect::Sqlite => "CAST(strftime('%s', updated_at) AS INTEGER)",
    }
}

fn revision_timestamp_sql(relational: &RelationalStorage, revision_unix: i64) -> String {
    match relational.dialect() {
        RelationalDialect::Postgres => format!("to_timestamp({revision_unix})"),
        RelationalDialect::Sqlite => format!("datetime({revision_unix}, 'unixepoch')"),
    }
}

fn parse_json_array_strings(value: Option<&Value>) -> Vec<String> {
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

fn parse_json_value_or_default(value: Option<&Value>, default: Value) -> Value {
    match value {
        Some(Value::String(raw)) => serde_json::from_str(raw).unwrap_or(default),
        Some(other) => other.clone(),
        None => default,
    }
}

fn parse_nullable_i32(value: Option<&Value>) -> Option<i32> {
    value.and_then(|value| {
        value
            .as_i64()
            .and_then(|raw| i32::try_from(raw).ok())
            .or_else(|| value.as_str().and_then(|raw| raw.parse::<i32>().ok()))
    })
}

fn parse_required_i32(value: Option<&Value>) -> i32 {
    parse_nullable_i32(value).unwrap_or_default()
}
