use anyhow::{Result as AnyhowResult, anyhow, bail};
use serde_json::{Number, Value};
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue};

use super::redaction::is_secret_path_segments;
use super::types::{REDACTED_VALUE, RuntimeConfigFieldPatchInput};

pub(super) fn apply_patch_to_document(
    doc: &mut DocumentMut,
    patch: RuntimeConfigFieldPatchInput,
) -> AnyhowResult<()> {
    if patch.path.is_empty() {
        bail!("patch path cannot be empty");
    }
    if patch
        .path
        .iter()
        .any(|segment| segment.trim().is_empty() || segment.contains('\0'))
    {
        bail!("patch path contains an invalid segment");
    }

    if patch.unset.unwrap_or(false) || patch.value.as_ref().is_none_or(|value| value.0.is_null()) {
        remove_path(doc, &patch.path);
        return Ok(());
    }

    let value = patch
        .value
        .map(|value| value.0)
        .ok_or_else(|| anyhow!("patch value is required unless unset is true"))?;
    if value == Value::String(REDACTED_VALUE.to_string()) && is_secret_path_segments(&patch.path) {
        return Ok(());
    }
    let value = toml_item_at_path(doc, &patch.path)
        .and_then(toml_item_to_json_value)
        .map(|original| preserve_redacted_placeholders(value.clone(), &original))
        .unwrap_or(value);
    set_path(doc, &patch.path, json_value_to_toml_item(&value)?)?;
    Ok(())
}

fn set_path(doc: &mut DocumentMut, path: &[String], value: Item) -> AnyhowResult<()> {
    if path.len() == 1 {
        doc[&path[0]] = value;
        return Ok(());
    }

    if doc.get(&path[0]).is_none_or(|item| !item.is_table()) {
        doc[&path[0]] = Item::Table(Table::new());
    }

    let mut table = doc[&path[0]]
        .as_table_mut()
        .ok_or_else(|| anyhow!("{} is not a table", path[0]))?;
    for segment in &path[1..path.len() - 1] {
        if table.get(segment).is_none_or(|item| !item.is_table()) {
            table[segment] = Item::Table(Table::new());
        }
        table = table[segment]
            .as_table_mut()
            .ok_or_else(|| anyhow!("{segment} is not a table"))?;
    }

    table[path.last().expect("path is non-empty")] = value;
    Ok(())
}

fn remove_path(doc: &mut DocumentMut, path: &[String]) {
    if path.len() == 1 {
        doc.as_table_mut().remove(&path[0]);
        return;
    }
    let Some(mut table) = doc.get_mut(&path[0]).and_then(Item::as_table_mut) else {
        return;
    };
    for segment in &path[1..path.len() - 1] {
        let Some(next) = table.get_mut(segment).and_then(Item::as_table_mut) else {
            return;
        };
        table = next;
    }
    if let Some(last) = path.last() {
        table.remove(last);
    }
}

fn json_value_to_toml_item(value: &Value) -> AnyhowResult<Item> {
    match value {
        Value::Null => Ok(Item::None),
        Value::Bool(value) => Ok(Item::Value(TomlValue::from(*value))),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                return Ok(Item::Value(TomlValue::from(value)));
            }
            if let Some(value) = number.as_u64() {
                return i64::try_from(value)
                    .map(|value| Item::Value(TomlValue::from(value)))
                    .map_err(|_| anyhow!("TOML integer value is too large: {number}"));
            }
            if let Some(value) = number.as_f64() {
                return Ok(Item::Value(TomlValue::from(value)));
            }
            bail!("unsupported numeric config value `{number}`")
        }
        Value::String(value) => Ok(Item::Value(TomlValue::from(value.as_str()))),
        Value::Array(values) => {
            let mut array = Array::new();
            for value in values {
                let Item::Value(value) = json_value_to_toml_item(value)? else {
                    bail!("TOML arrays may only contain scalar values")
                };
                array.push(value);
            }
            Ok(Item::Value(TomlValue::Array(array)))
        }
        Value::Object(map) => {
            let mut table = Table::new();
            for (key, value) in map {
                table[key] = json_value_to_toml_item(value)?;
            }
            Ok(Item::Table(table))
        }
    }
}

fn toml_item_at_path<'a>(doc: &'a DocumentMut, path: &[String]) -> Option<&'a Item> {
    let mut item = doc.get(path.first()?)?;
    for segment in &path[1..] {
        item = item.as_table()?.get(segment)?;
    }
    Some(item)
}

fn toml_item_to_json_value(item: &Item) -> Option<Value> {
    match item {
        Item::None => Some(Value::Null),
        Item::Value(value) => Some(toml_value_to_json_value(value)),
        Item::Table(table) => Some(Value::Object(
            table
                .iter()
                .filter_map(|(key, item)| {
                    toml_item_to_json_value(item).map(|value| (key.to_string(), value))
                })
                .collect(),
        )),
        Item::ArrayOfTables(tables) => Some(Value::Array(
            tables
                .iter()
                .map(|table| {
                    Value::Object(
                        table
                            .iter()
                            .filter_map(|(key, item)| {
                                toml_item_to_json_value(item).map(|value| (key.to_string(), value))
                            })
                            .collect(),
                    )
                })
                .collect(),
        )),
    }
}

fn toml_value_to_json_value(value: &TomlValue) -> Value {
    if let Some(value) = value.as_str() {
        return Value::String(value.to_string());
    }
    if let Some(value) = value.as_integer() {
        return Value::Number(value.into());
    }
    if let Some(value) = value.as_float() {
        return Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(value.to_string()));
    }
    if let Some(value) = value.as_bool() {
        return Value::Bool(value);
    }
    if let Some(value) = value.as_datetime() {
        return Value::String(value.to_string());
    }
    if let Some(array) = value.as_array() {
        return Value::Array(array.iter().map(toml_value_to_json_value).collect());
    }
    if let Some(table) = value.as_inline_table() {
        return Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.to_string(), toml_value_to_json_value(value)))
                .collect(),
        );
    }
    Value::Null
}

fn preserve_redacted_placeholders(next: Value, original: &Value) -> Value {
    match (next, original) {
        (Value::String(value), original) if value == REDACTED_VALUE => original.clone(),
        (Value::Object(next), Value::Object(original)) => Value::Object(
            next.into_iter()
                .map(|(key, value)| {
                    let value = original
                        .get(&key)
                        .map(|original| preserve_redacted_placeholders(value.clone(), original))
                        .unwrap_or(value);
                    (key, value)
                })
                .collect(),
        ),
        (Value::Array(next), Value::Array(original)) => Value::Array(
            next.into_iter()
                .enumerate()
                .map(|(index, value)| {
                    original
                        .get(index)
                        .map(|original| preserve_redacted_placeholders(value.clone(), original))
                        .unwrap_or(value)
                })
                .collect(),
        ),
        (next, _) => next,
    }
}

#[cfg(test)]
mod tests {
    use async_graphql::types::Json;
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn apply_patch_preserves_unrelated_toml_and_updates_nested_value() {
        let original = r#"# keep me
[runtime]
local_dev = false

[stores.relational]
sqlite_path = "old.db"
"#;
        let mut doc = original.parse::<DocumentMut>().expect("parse toml");
        apply_patch_to_document(
            &mut doc,
            RuntimeConfigFieldPatchInput {
                path: vec![
                    "stores".to_string(),
                    "relational".to_string(),
                    "sqlite_path".to_string(),
                ],
                value: Some(Json(Value::String("new.db".to_string()))),
                unset: None,
            },
        )
        .expect("apply patch");
        let updated = doc.to_string();
        assert!(updated.contains("# keep me"));
        assert!(updated.contains("local_dev = false"));
        assert!(updated.contains("sqlite_path = \"new.db\""));
    }

    #[test]
    fn apply_patch_preserves_redacted_nested_secret_values() {
        let original = r#"[knowledge.providers.github]
token = "secret-token"
enabled = true
"#;
        let mut doc = original.parse::<DocumentMut>().expect("parse toml");
        apply_patch_to_document(
            &mut doc,
            RuntimeConfigFieldPatchInput {
                path: vec!["knowledge".to_string(), "providers".to_string()],
                value: Some(Json(json!({
                    "github": {
                        "token": REDACTED_VALUE,
                        "enabled": false,
                    },
                }))),
                unset: None,
            },
        )
        .expect("apply patch");
        let updated = doc.to_string();
        assert!(updated.contains("token = \"secret-token\""));
        assert!(updated.contains("enabled = false"));
    }
}
