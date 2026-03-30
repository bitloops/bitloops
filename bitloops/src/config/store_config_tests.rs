pub(crate) use super::*;
pub(crate) use crate::test_support::process_state::{
    enter_process_state, with_cwd, with_process_state,
};
pub(crate) use serde_json::Value;
pub(crate) use std::fs;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue};

mod backend;
mod blob;
mod dashboard;
mod embedding;
mod events;
mod knowledge_providers;
mod providerless;
mod semantic;
mod sqlite_path;
mod watch;

fn json_value_to_toml_item(value: &Value) -> Item {
    match value {
        Value::Null => Item::None,
        Value::Bool(value) => Item::Value(TomlValue::from(*value)),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Item::Value(TomlValue::from(value))
            } else if let Some(value) = number.as_u64() {
                Item::Value(TomlValue::from(value as i64))
            } else if let Some(value) = number.as_f64() {
                Item::Value(TomlValue::from(value))
            } else {
                panic!("unsupported numeric test config value: {number}");
            }
        }
        Value::String(value) => Item::Value(TomlValue::from(value.as_str())),
        Value::Array(values) => {
            let mut array = Array::new();
            for value in values {
                let Item::Value(value) = json_value_to_toml_item(value) else {
                    panic!("test config arrays must contain scalar values");
                };
                array.push(value);
            }
            Item::Value(TomlValue::Array(array))
        }
        Value::Object(map) => {
            let mut table = Table::new();
            for (key, value) in map {
                table[key] = json_value_to_toml_item(value);
            }
            Item::Table(table)
        }
    }
}

pub(crate) fn write_repo_config(repo_root: &Path, value: serde_json::Value) {
    let config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let parent = config_path.parent().expect("config parent");
    fs::create_dir_all(parent).expect("create config dir");
    let mut doc = DocumentMut::new();
    for (key, value) in value.as_object().expect("top-level config object") {
        doc[key] = json_value_to_toml_item(value);
    }
    fs::write(&config_path, doc.to_string()).expect("write config");
}

pub(crate) fn write_envelope_config(repo_root: &Path, settings: serde_json::Value) {
    write_repo_config(repo_root, settings);
}
