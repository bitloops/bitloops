use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result as AnyhowResult};
use async_graphql::types::Json;
use serde_json::{Value, json};
use toml_edit::de::from_str;

use super::identity::revision_for_bytes;
use super::redaction::redact_json_value;
use super::sections::build_sections_for_target;
use super::types::{ConfigTarget, ConfigTargetKind, RuntimeConfigSnapshotObject};
use super::validation::validate_target_text;

pub(super) fn build_snapshot(target: &ConfigTarget) -> AnyhowResult<RuntimeConfigSnapshotObject> {
    let raw = fs::read_to_string(&target.path)
        .with_context(|| format!("reading config target {}", target.path.display()))?;
    let revision = revision_for_bytes(raw.as_bytes());
    let value = parse_toml_value(&raw, &target.path)?;
    let redacted_value = redact_json_value(&value);
    let validation = validate_target_text(target, &raw)
        .map(|_| Vec::new())
        .unwrap_or_else(|err| vec![format!("{err:#}")]);
    let effective = effective_value_for_target(target, &value);
    let sections = build_sections_for_target(target, &value, effective.as_ref());

    Ok(RuntimeConfigSnapshotObject {
        target: target.clone().into(),
        revision,
        valid: validation.is_empty(),
        validation_errors: validation,
        restart_required: target.kind == ConfigTargetKind::Daemon,
        reload_required: target.kind != ConfigTargetKind::Daemon,
        sections,
        raw_value: Json(redacted_value),
        effective_value: effective.map(|value| Json(redact_json_value(&value))),
    })
}

fn parse_toml_value(raw: &str, path: &Path) -> AnyhowResult<Value> {
    from_str::<Value>(raw).with_context(|| format!("parsing config target {}", path.display()))
}

fn effective_value_for_target(target: &ConfigTarget, value: &Value) -> Option<Value> {
    match target.kind {
        ConfigTargetKind::Daemon => Some(value.clone()),
        ConfigTargetKind::RepoShared | ConfigTargetKind::RepoLocal => {
            let root = target.path.parent()?;
            crate::config::discover_repo_policy_optional(root)
                .ok()
                .map(repo_policy_snapshot_to_value)
        }
    }
}

fn repo_policy_snapshot_to_value(snapshot: crate::config::RepoPolicySnapshot) -> Value {
    json!({
        "capture": snapshot.capture,
        "watch": snapshot.watch,
        "scope": snapshot.scope,
        "contexts": snapshot.contexts,
        "agents": snapshot.agents,
        "imports": {
            "knowledge": snapshot.knowledge_import_paths,
        },
        "daemon": {
            "config_path": snapshot.daemon_config_path.map(|path| path.display().to_string()),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::config::BITLOOPS_CONFIG_RELATIVE_PATH;

    use super::super::identity::target_id;
    use super::super::types::REDACTED_VALUE;
    use super::*;

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, text).expect("write file");
    }

    #[test]
    fn snapshot_redacts_secret_values() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = temp.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
        write(
            &path,
            r#"[runtime]
local_dev = false

[knowledge.providers.github]
token = "secret-token"
"#,
        );
        let target = ConfigTarget {
            id: target_id("daemon", &path),
            kind: ConfigTargetKind::Daemon,
            label: "Daemon config".to_string(),
            group: "Daemon".to_string(),
            path,
            repo_root: None,
            exists: true,
        };

        let snapshot = build_snapshot(&target).expect("snapshot");
        assert_eq!(
            snapshot.raw_value.0["knowledge"]["providers"]["github"]["token"],
            Value::String(REDACTED_VALUE.to_string())
        );
    }
}
