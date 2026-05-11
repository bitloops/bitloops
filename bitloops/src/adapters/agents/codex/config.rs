use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use toml_edit::{DocumentMut, Item, Table, Value as TomlValue};

const CONFIG_FILE_NAME: &str = "config.toml";
const FEATURES_TABLE_NAME: &str = "features";
const CANONICAL_HOOKS_FEATURE_KEY: &str = "hooks";
const LEGACY_HOOKS_FEATURE_KEY: &str = "codex_hooks";

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

    let features = ensure_features_table(&mut doc)?;
    let canonical_enabled = feature_flag(features, CANONICAL_HOOKS_FEATURE_KEY).unwrap_or(false);
    let legacy_present = features.contains_key(LEGACY_HOOKS_FEATURE_KEY);
    let mut changed = false;

    if !canonical_enabled {
        features[CANONICAL_HOOKS_FEATURE_KEY] = Item::Value(TomlValue::from(true));
        changed = true;
    }
    if legacy_present {
        features.remove(LEGACY_HOOKS_FEATURE_KEY);
        changed = true;
    }

    if !changed {
        return Ok(path);
    }

    write_config(&path, doc)?;
    Ok(path)
}

pub fn codex_hooks_feature_enabled_at(repo_root: &Path) -> bool {
    let path = codex_config_path(repo_root);
    let Ok(existing) = fs::read_to_string(&path) else {
        return false;
    };
    let Ok(doc) = existing.parse::<DocumentMut>() else {
        return false;
    };
    codex_hooks_feature_enabled(&doc)
}

fn codex_hooks_feature_enabled(doc: &DocumentMut) -> bool {
    let Some(features) = doc.get(FEATURES_TABLE_NAME).and_then(Item::as_table) else {
        return false;
    };

    feature_flag(features, CANONICAL_HOOKS_FEATURE_KEY)
        .or_else(|| feature_flag(features, LEGACY_HOOKS_FEATURE_KEY))
        .unwrap_or(false)
}

fn feature_flag(features: &Table, key: &str) -> Option<bool> {
    features.get(key).and_then(Item::as_bool)
}

fn ensure_features_table(doc: &mut DocumentMut) -> Result<&mut Table> {
    let root = doc.as_table_mut();
    if let Some(existing) = root.get(FEATURES_TABLE_NAME)
        && !existing.is_table()
    {
        return Err(anyhow!(
            "refusing to overwrite non-table `features` in Codex config"
        ));
    }
    if !root.contains_key(FEATURES_TABLE_NAME) || !root[FEATURES_TABLE_NAME].is_table() {
        root.insert(FEATURES_TABLE_NAME, Item::Table(Table::new()));
    }
    Ok(root[FEATURES_TABLE_NAME]
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
