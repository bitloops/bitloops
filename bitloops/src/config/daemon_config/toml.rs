use toml_edit::{DocumentMut, Item, Table};

pub(super) fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
    let root = doc.as_table_mut();
    if !root.contains_key(key) || !root[key].is_table() {
        root.insert(key, Item::Table(Table::new()));
    }
    root[key]
        .as_table_mut()
        .expect("TOML item should be a table after initialisation")
}

pub(super) fn ensure_child_table<'a>(table: &'a mut Table, key: &str) -> &'a mut Table {
    if !table.contains_key(key) || !table[key].is_table() {
        table.insert(key, Item::Table(Table::new()));
    }
    table[key]
        .as_table_mut()
        .expect("TOML item should be a table after initialisation")
}

/// Reads legacy daemon-global semantic embedding bindings for migration and
/// warm-existing installer behavior.
pub(super) fn selected_inference_profile_name(doc: &DocumentMut) -> Option<String> {
    let inference = doc
        .as_table()
        .get("semantic_clones")?
        .as_table()?
        .get("inference")?
        .as_table()?;

    for key in ["code_embeddings", "summary_embeddings"] {
        let Some(value) = inference
            .get(key)
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
        else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        if matches!(
            value.to_ascii_lowercase().as_str(),
            "none" | "disabled" | "off"
        ) {
            continue;
        }
        return Some(value.to_string());
    }

    None
}

pub(super) fn inference_driver_for_profile(
    doc: &DocumentMut,
    profile_name: &str,
) -> Option<String> {
    inference_profile_value(doc, profile_name, "driver")
}

pub(super) fn inference_runtime_for_profile(
    doc: &DocumentMut,
    profile_name: &str,
) -> Option<String> {
    inference_profile_value(doc, profile_name, "runtime")
}

fn inference_profile_value(doc: &DocumentMut, profile_name: &str, key: &str) -> Option<String> {
    doc.as_table()
        .get("inference")?
        .as_table()?
        .get("profiles")?
        .as_table()?
        .get(profile_name)?
        .as_table()?
        .get(key)?
        .as_value()?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
