use serde_json::Value;

pub(super) fn normalize_scope_exclusion_array_literals(raw: &str) -> Option<String> {
    let mut changed = false;
    let mut lines = Vec::new();

    for line in raw.lines() {
        let normalized = normalize_scope_exclusion_array_line(line);
        if normalized != line {
            changed = true;
        }
        lines.push(normalized);
    }

    if !changed {
        return None;
    }

    let mut out = lines.join("\n");
    if raw.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

fn normalize_scope_exclusion_array_line(line: &str) -> String {
    if line.contains('#') {
        return line.to_string();
    }

    let Some((lhs, rhs)) = line.split_once('=') else {
        return line.to_string();
    };
    let key = lhs.trim();
    if key != "exclude" && key != "exclude_from" {
        return line.to_string();
    }

    let rhs = rhs.trim();
    if !(rhs.starts_with('[') && rhs.ends_with(']')) {
        return line.to_string();
    }

    let body = &rhs[1..rhs.len().saturating_sub(1)];
    let values = split_array_values(body);
    if values.is_empty() {
        return line.to_string();
    }

    let mut changed = false;
    let mut normalized_values = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if is_quoted_string_literal(value) {
            normalized_values.push(value.to_string());
            continue;
        }

        normalized_values.push(format!("\"{}\"", escape_toml_basic_string(value)));
        changed = true;
    }

    if !changed {
        return line.to_string();
    }

    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    format!("{indent}{key} = [{}]", normalized_values.join(", "))
}

fn split_array_values(input: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if in_double_quotes && ch == '\\' {
            current.push(ch);
            escaped = true;
            continue;
        }

        match ch {
            '\'' if !in_double_quotes => {
                in_single_quotes = !in_single_quotes;
                current.push(ch);
            }
            '"' if !in_single_quotes => {
                in_double_quotes = !in_double_quotes;
                current.push(ch);
            }
            ',' if !in_single_quotes && !in_double_quotes => {
                values.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }

    values.push(current);
    values
}

fn is_quoted_string_literal(value: &str) -> bool {
    value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
}

fn escape_toml_basic_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(super) fn merge_optional_values(base: Option<Value>, overlay: Option<Value>) -> Value {
    match (base, overlay) {
        (Some(base), Some(overlay)) => deep_merge_value(base, overlay),
        (Some(base), None) => base,
        (None, Some(overlay)) => overlay,
        (None, None) => Value::Object(serde_json::Map::new()),
    }
}

pub(super) fn merge_scope_values(base: Option<Value>, overlay: Option<Value>) -> Value {
    match (base, overlay) {
        (Some(base), Some(overlay)) => {
            if scope_overlay_replaces_exclusions(&overlay) {
                deep_merge_value(remove_scope_exclusion_keys(base), overlay)
            } else {
                deep_merge_value(base, overlay)
            }
        }
        (Some(base), None) => base,
        (None, Some(overlay)) => overlay,
        (None, None) => Value::Object(serde_json::Map::new()),
    }
}

fn scope_overlay_replaces_exclusions(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|map| map.contains_key("exclude") || map.contains_key("exclude_from"))
}

fn remove_scope_exclusion_keys(value: Value) -> Value {
    if let Value::Object(mut map) = value {
        map.remove("exclude");
        map.remove("exclude_from");
        Value::Object(map)
    } else {
        value
    }
}

pub(super) fn deep_merge_value(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            let mut merged = base_map;
            for (key, overlay_value) in overlay_map {
                match (merged.remove(&key), overlay_value) {
                    (_, Value::Null) => {}
                    (Some(existing), overlay_value) => {
                        merged.insert(key, deep_merge_value(existing, overlay_value));
                    }
                    (None, overlay_value) => {
                        merged.insert(key, overlay_value);
                    }
                }
            }
            Value::Object(merged)
        }
        (_, overlay) => overlay,
    }
}
