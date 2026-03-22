use super::*;

pub(crate) fn normalise_relational_result_row(row: Value) -> Value {
    let Some(mut obj) = row.as_object().cloned() else {
        return row;
    };

    for key in ["modifiers", "metadata", "explanation_json"] {
        if let Some(raw) = obj.get(key).and_then(Value::as_str)
            && let Ok(parsed) = serde_json::from_str::<Value>(raw)
        {
            obj.insert(key.to_string(), parsed);
        }
    }

    if let Some(edge_kind) = obj.get("edge_kind").and_then(Value::as_str)
        && let Some(normalized) = normalise_edge_kind_value(edge_kind)
    {
        obj.insert("edge_kind".to_string(), Value::String(normalized.clone()));
        if let Some(metadata) = obj.get_mut("metadata") {
            normalise_edge_metadata(&normalized, metadata);
        }
    }

    Value::Object(obj)
}
