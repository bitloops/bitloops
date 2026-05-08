use anyhow::{Context, Result};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::types::RepoPolicyFingerprintInputs;

pub(super) fn fingerprint_repo_policy(inputs: RepoPolicyFingerprintInputs<'_>) -> Result<String> {
    let RepoPolicyFingerprintInputs {
        capture,
        watch,
        devql,
        scope,
        scope_exclusions,
        contexts,
        agents,
        knowledge_import_paths,
        imported_knowledge,
    } = inputs;
    let mut root = Map::new();
    root.insert("capture".into(), canonicalize_value(capture));
    root.insert("watch".into(), canonicalize_value(watch));
    root.insert("devql".into(), canonicalize_value(devql));
    root.insert("scope".into(), canonicalize_value(scope));
    root.insert(
        "scope_exclusions".into(),
        Value::Object(Map::from_iter([
            (
                "exclude".into(),
                Value::Array(
                    scope_exclusions
                        .exclude
                        .iter()
                        .map(|value| Value::String(value.clone()))
                        .collect(),
                ),
            ),
            (
                "exclude_from".into(),
                Value::Array(
                    scope_exclusions
                        .exclude_from
                        .iter()
                        .map(|value| Value::String(value.clone()))
                        .collect(),
                ),
            ),
            (
                "exclude_from_files".into(),
                Value::Array(
                    scope_exclusions
                        .referenced_files
                        .iter()
                        .map(|entry| {
                            Value::Object(Map::from_iter([
                                (
                                    "configured_path".into(),
                                    Value::String(entry.configured_path.clone()),
                                ),
                                (
                                    "resolved_path".into(),
                                    Value::String(
                                        entry.resolved_path.to_string_lossy().to_string(),
                                    ),
                                ),
                                ("content".into(), Value::String(entry.content.clone())),
                                (
                                    "patterns".into(),
                                    Value::Array(
                                        entry
                                            .patterns
                                            .iter()
                                            .map(|value| Value::String(value.clone()))
                                            .collect(),
                                    ),
                                ),
                            ]))
                        })
                        .collect(),
                ),
            ),
        ])),
    );
    root.insert("contexts".into(), canonicalize_value(contexts));
    root.insert("agents".into(), canonicalize_value(agents));
    root.insert(
        "imports".into(),
        Value::Object(Map::from_iter([(
            "knowledge".into(),
            Value::Array(
                knowledge_import_paths
                    .iter()
                    .map(|path| Value::String(path.clone()))
                    .collect(),
            ),
        )])),
    );
    root.insert(
        "knowledge".into(),
        Value::Array(
            imported_knowledge
                .iter()
                .map(|knowledge| {
                    Value::Object(Map::from_iter([
                        (
                            "path".into(),
                            Value::String(knowledge.path.to_string_lossy().to_string()),
                        ),
                        ("config".into(), canonicalize_value(&knowledge.value)),
                    ]))
                })
                .collect(),
        ),
    );

    let bytes = serde_json::to_vec(&Value::Object(root)).context("serialising repo policy")?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn canonicalize_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                if let Some(value) = map.get(&key) {
                    out.insert(key, canonicalize_value(value));
                }
            }
            Value::Object(out)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_value).collect()),
        _ => value.clone(),
    }
}
