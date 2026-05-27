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

pub(super) fn strip_semantic_enablement(doc: &mut DocumentMut) -> bool {
    let Some(semantic_item) = doc.as_table_mut().get_mut("semantic_clones") else {
        return false;
    };
    let Some(semantic) = semantic_item.as_table_mut() else {
        return false;
    };

    let mut modified = false;
    for key in ["summary_mode", "embedding_mode"] {
        modified |= semantic.remove(key).is_some();
    }

    if let Some(inference) = semantic.get_mut("inference").and_then(Item::as_table_mut) {
        for key in [
            "summary_generation",
            "code_embeddings",
            "summary_embeddings",
        ] {
            modified |= inference.remove(key).is_some();
        }
    }
    if semantic
        .get("inference")
        .and_then(Item::as_table)
        .is_some_and(Table::is_empty)
    {
        semantic.remove("inference");
        modified = true;
    }
    if semantic.is_empty() {
        doc.as_table_mut().remove("semantic_clones");
        modified = true;
    }

    modified
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
