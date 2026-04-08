use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use toml_edit::{DocumentMut, Item, Table, Value as TomlValue};

const CONFIG_FILE_NAME: &str = "config.toml";

pub fn codex_config_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".codex").join(CONFIG_FILE_NAME)
}

pub fn ensure_codex_hooks_feature_enabled_at(repo_root: &Path) -> Result<PathBuf> {
    let path = codex_config_path(repo_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| anyhow!("failed to create .codex directory: {err}"))?;
    }

    let mut doc = match fs::read_to_string(&path) {
        Ok(existing) => existing
            .parse::<DocumentMut>()
            .with_context(|| format!("failed to parse Codex config {}", path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(err) => {
            return Err(err).with_context(|| format!("reading Codex config {}", path.display()));
        }
    };

    if codex_hooks_feature_enabled(&doc) {
        return Ok(path);
    }

    let features = ensure_features_table(&mut doc)?;
    features["codex_hooks"] = Item::Value(TomlValue::from(true));
    write_config(&path, doc)?;
    Ok(path)
}

fn codex_hooks_feature_enabled(doc: &DocumentMut) -> bool {
    doc.get("features")
        .and_then(Item::as_table)
        .and_then(|features| features.get("codex_hooks"))
        .and_then(Item::as_bool)
        .unwrap_or(false)
}

fn ensure_features_table(doc: &mut DocumentMut) -> Result<&mut Table> {
    let root = doc.as_table_mut();
    if let Some(existing) = root.get("features")
        && !existing.is_table()
    {
        return Err(anyhow!(
            "refusing to overwrite non-table `features` in Codex config"
        ));
    }
    if !root.contains_key("features") || !root["features"].is_table() {
        root.insert("features", Item::Table(Table::new()));
    }
    Ok(root["features"]
        .as_table_mut()
        .expect("features should be a TOML table"))
}

fn write_config(path: &Path, doc: DocumentMut) -> Result<()> {
    let mut output = doc.to_string();
    if !output.ends_with('\n') {
        output.push('\n');
    }
    fs::write(path, output).with_context(|| format!("writing Codex config {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
