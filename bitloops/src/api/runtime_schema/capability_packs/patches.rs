use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result as AnyhowResult, anyhow, bail};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue, de::from_str};

use super::models::{
    ARCHITECTURE_FACT_SYNTHESIS_SLOT, ARCHITECTURE_ROLE_ADJUDICATION_SLOT,
    CONTEXT_GUIDANCE_GENERATION_SLOT, CapabilityPackFieldPatchInput, DraftPatch, LoadedConfigFile,
    PlanCapabilityPackConfigInput, PlannerTargetKind, STRUCTURED_GENERATION_TASK,
    TEXT_GENERATION_TASK,
};
use super::values::{read_string_list, value_at_path};
use crate::host::inference::BITLOOPS_INFERENCE_RUNTIME_ID;

pub(super) fn build_combined_patches(
    input: &PlanCapabilityPackConfigInput,
    daemon_value: &Value,
) -> AnyhowResult<Vec<DraftPatch>> {
    let mut daemon_patches = Vec::new();

    daemon_patches.push(DraftPatch {
        target: PlannerTargetKind::Daemon,
        path: vec![
            "runtime".to_string(),
            "capability_policy".to_string(),
            "explicit_enabled".to_string(),
        ],
        value: Some(Value::Array(
            input
                .explicit_enabled
                .iter()
                .map(|value| Value::String(value.trim().to_string()))
                .collect(),
        )),
        unset: false,
    });
    daemon_patches.push(DraftPatch {
        target: PlannerTargetKind::Daemon,
        path: vec![
            "runtime".to_string(),
            "capability_policy".to_string(),
            "explicit_disabled".to_string(),
        ],
        value: Some(Value::Array(
            input
                .explicit_disabled
                .iter()
                .map(|value| Value::String(value.trim().to_string()))
                .collect(),
        )),
        unset: false,
    });

    if enabled_pack_ids(&input.explicit_enabled, daemon_value).contains("architecture_graph") {
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "architecture",
                "inference",
                ARCHITECTURE_FACT_SYNTHESIS_SLOT,
            ],
            json!("architecture_fact_synthesis_managed"),
            daemon_value,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "architecture",
                "inference",
                ARCHITECTURE_ROLE_ADJUDICATION_SLOT,
            ],
            json!("architecture_role_adjudication_managed"),
            daemon_value,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "profiles",
                "architecture_fact_synthesis_managed",
                "task",
            ],
            json!(STRUCTURED_GENERATION_TASK),
            daemon_value,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "profiles",
                "architecture_role_adjudication_managed",
                "task",
            ],
            json!(STRUCTURED_GENERATION_TASK),
            daemon_value,
        ));
    }
    if enabled_pack_ids(&input.explicit_enabled, daemon_value).contains("context_guidance") {
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "context_guidance",
                "inference",
                CONTEXT_GUIDANCE_GENERATION_SLOT,
            ],
            json!("guidance_llm"),
            daemon_value,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &["inference", "profiles", "guidance_llm", "task"],
            json!(TEXT_GENERATION_TASK),
            daemon_value,
        ));
    }

    daemon_patches.retain(|patch| !patch.unset || patch.value.is_some());
    for patch in &input.daemon_patches {
        daemon_patches.push(to_draft_patch(PlannerTargetKind::Daemon, patch)?);
    }

    let daemon_preview = apply_patches_to_json(daemon_value, &daemon_patches)?;
    for runtime_name in structured_runtime_names(&daemon_preview) {
        if matches!(runtime_name.as_str(), "codex" | "claude")
            && let Some((command, args)) = probe_local_runtime(runtime_name.as_str())
        {
            daemon_patches.push(seed_if_missing(
                PlannerTargetKind::Daemon,
                &["inference", "runtimes", runtime_name.as_str(), "command"],
                Value::String(command),
                &daemon_preview,
            ));
            daemon_patches.push(seed_if_missing(
                PlannerTargetKind::Daemon,
                &["inference", "runtimes", runtime_name.as_str(), "args"],
                Value::Array(args.into_iter().map(Value::String).collect()),
                &daemon_preview,
            ));
            daemon_patches.push(seed_if_missing(
                PlannerTargetKind::Daemon,
                &[
                    "inference",
                    "runtimes",
                    runtime_name.as_str(),
                    "startup_timeout_secs",
                ],
                json!(5),
                &daemon_preview,
            ));
            daemon_patches.push(seed_if_missing(
                PlannerTargetKind::Daemon,
                &[
                    "inference",
                    "runtimes",
                    runtime_name.as_str(),
                    "request_timeout_secs",
                ],
                json!(900),
                &daemon_preview,
            ));
        }
    }
    if requires_managed_bitloops_inference(&daemon_preview) {
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "runtimes",
                BITLOOPS_INFERENCE_RUNTIME_ID,
                "command",
            ],
            json!("bitloops-inference"),
            &daemon_preview,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "runtimes",
                BITLOOPS_INFERENCE_RUNTIME_ID,
                "args",
            ],
            json!([]),
            &daemon_preview,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "runtimes",
                BITLOOPS_INFERENCE_RUNTIME_ID,
                "startup_timeout_secs",
            ],
            json!(60),
            &daemon_preview,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "runtimes",
                BITLOOPS_INFERENCE_RUNTIME_ID,
                "request_timeout_secs",
            ],
            json!(300),
            &daemon_preview,
        ));
    }

    Ok(dedupe_patches(daemon_patches))
}

fn enabled_pack_ids(explicit_enabled: &[String], daemon_value: &Value) -> BTreeSet<String> {
    if explicit_enabled.is_empty() {
        read_string_list(
            daemon_value,
            &["runtime", "capability_policy", "explicit_enabled"],
        )
        .into_iter()
        .collect()
    } else {
        explicit_enabled
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    }
}

fn structured_runtime_names(value: &Value) -> BTreeSet<String> {
    let Some(profiles) = value
        .get("inference")
        .and_then(|value| value.get("profiles"))
        .and_then(Value::as_object)
    else {
        return BTreeSet::new();
    };
    profiles
        .iter()
        .filter(|(_, profile)| {
            matches!(
                profile.get("driver").and_then(Value::as_str),
                Some("codex_exec" | "claude_code_print")
            )
        })
        .filter_map(|(_, profile)| profile.get("runtime").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn requires_managed_bitloops_inference(value: &Value) -> bool {
    let Some(profiles) = value
        .get("inference")
        .and_then(|value| value.get("profiles"))
        .and_then(Value::as_object)
    else {
        return false;
    };
    profiles.values().any(|profile| {
        matches!(
            profile.get("driver").and_then(Value::as_str),
            Some("codex_exec" | "claude_code_print")
        )
    })
}

fn probe_local_runtime(runtime_name: &str) -> Option<(String, Vec<String>)> {
    let candidates = match runtime_name {
        "codex" => vec!["codex", "/opt/homebrew/bin/codex", "/usr/local/bin/codex"],
        "claude" => vec![
            "claude",
            "/opt/homebrew/bin/claude",
            "/usr/local/bin/claude",
        ],
        _ => Vec::new(),
    };
    for candidate in candidates {
        let path = if candidate.contains('/') {
            let path = PathBuf::from(candidate);
            if path.is_file() { Some(path) } else { None }
        } else {
            resolve_command_on_path(candidate)
        };
        if let Some(path) = path {
            return Some((path.display().to_string(), Vec::new()));
        }
    }
    None
}

fn resolve_command_on_path(program: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|dir| {
        let direct = dir.join(program);
        if direct.is_file() {
            return Some(direct);
        }
        executable_with_extensions(&dir, program)
            .into_iter()
            .find(|candidate| candidate.is_file())
    })
}

fn executable_with_extensions(dir: &Path, program: &str) -> [PathBuf; 3] {
    [
        dir.join(format!("{program}.exe")),
        dir.join(format!("{program}.cmd")),
        dir.join(format!("{program}.bat")),
    ]
}

fn to_draft_patch(
    target: PlannerTargetKind,
    patch: &CapabilityPackFieldPatchInput,
) -> AnyhowResult<DraftPatch> {
    if !patch.target.trim().is_empty() && patch.target.trim() != target.as_str() {
        bail!(
            "capability config patches are daemon-scoped; unsupported target `{}`",
            patch.target
        );
    }
    Ok(DraftPatch {
        target,
        path: patch.path.clone(),
        value: patch.value.as_ref().map(|value| value.0.clone()),
        unset: patch.unset.unwrap_or(false)
            || patch.value.as_ref().is_some_and(|value| value.0.is_null()),
    })
}

fn dedupe_patches(patches: Vec<DraftPatch>) -> Vec<DraftPatch> {
    let mut by_key = BTreeMap::<String, DraftPatch>::new();
    for patch in patches {
        let key = format!("{}:{}", patch.target.as_str(), patch.path.join("."));
        by_key.insert(key, patch);
    }
    by_key.into_values().collect()
}

fn seed_if_missing(
    target: PlannerTargetKind,
    path: &[&str],
    value: Value,
    current: &Value,
) -> DraftPatch {
    let path_segments = path
        .iter()
        .map(|segment| (*segment).to_string())
        .collect::<Vec<_>>();
    let exists = value_at_path(current, path).is_some_and(|value| !value.is_null());
    DraftPatch {
        target,
        path: path_segments,
        value: (!exists).then_some(value),
        unset: false,
    }
}

pub(super) fn load_daemon_file(path: PathBuf) -> AnyhowResult<LoadedConfigFile> {
    load_config_file(path, true)
}

fn load_config_file(path: PathBuf, require_exists: bool) -> AnyhowResult<LoadedConfigFile> {
    let raw_text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && !require_exists => String::new(),
        Err(err) => {
            return Err(err).with_context(|| format!("reading config {}", path.display()));
        }
    };
    let value = if raw_text.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        from_str::<Value>(&raw_text)
            .with_context(|| format!("parsing config {}", path.display()))?
    };
    let revision = (!raw_text.is_empty()).then(|| revision_for_bytes(raw_text.as_bytes()));
    Ok(LoadedConfigFile {
        path,
        raw_text,
        value,
        revision,
    })
}

pub(super) fn apply_patches_to_json(root: &Value, patches: &[DraftPatch]) -> AnyhowResult<Value> {
    let mut next = root.clone();
    for patch in patches {
        if patch.unset || patch.value.as_ref().is_none_or(Value::is_null) {
            remove_json_path(&mut next, &patch.path);
            continue;
        }
        set_json_path(
            &mut next,
            &patch.path,
            patch
                .value
                .clone()
                .ok_or_else(|| anyhow!("patch value is required"))?,
        )?;
    }
    Ok(next)
}

fn set_json_path(root: &mut Value, path: &[String], value: Value) -> AnyhowResult<()> {
    if path.is_empty() {
        bail!("json patch path cannot be empty");
    }
    if !root.is_object() {
        *root = Value::Object(Map::new());
    }
    let mut current = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("config root must be an object"))?;
    for segment in &path[..path.len() - 1] {
        if current.get(segment).is_none_or(|value| !value.is_object()) {
            current.insert(segment.clone(), Value::Object(Map::new()));
        }
        current = current
            .get_mut(segment)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("failed to create nested config object for `{segment}`"))?;
    }
    current.insert(path.last().expect("path is non-empty").clone(), value);
    Ok(())
}

fn remove_json_path(root: &mut Value, path: &[String]) {
    if path.is_empty() {
        return;
    }
    let Some(mut current) = root.as_object_mut() else {
        return;
    };
    for segment in &path[..path.len() - 1] {
        let Some(next) = current.get_mut(segment).and_then(Value::as_object_mut) else {
            return;
        };
        current = next;
    }
    if let Some(last) = path.last() {
        current.remove(last);
    }
}

pub(super) fn apply_patches_to_toml(
    original: &str,
    patches: &[DraftPatch],
) -> AnyhowResult<String> {
    let mut doc = if original.trim().is_empty() {
        DocumentMut::new()
    } else {
        original.parse::<DocumentMut>()?
    };
    for patch in patches {
        apply_patch_to_document(&mut doc, patch.clone())?;
    }
    Ok(doc.to_string())
}

fn apply_patch_to_document(doc: &mut DocumentMut, patch: DraftPatch) -> AnyhowResult<()> {
    if patch.path.is_empty() {
        bail!("patch path cannot be empty");
    }
    if patch.unset || patch.value.as_ref().is_none_or(Value::is_null) {
        remove_path(doc, &patch.path);
        return Ok(());
    }
    set_path(
        doc,
        &patch.path,
        json_value_to_toml_item(
            &patch
                .value
                .ok_or_else(|| anyhow!("patch value is required"))?,
        )?,
    )
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
                    bail!("TOML arrays may only contain scalar values");
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

pub(super) fn hash_plan(
    input: &PlanCapabilityPackConfigInput,
    daemon_revision: &str,
    daemon_preview: &Value,
) -> AnyhowResult<String> {
    let payload = serde_json::to_vec(&json!({
        "explicit_enabled": input.explicit_enabled,
        "explicit_disabled": input.explicit_disabled,
        "daemon_patches": input.daemon_patches.iter().map(|patch| json!({
            "target": patch.target,
            "path": patch.path,
            "value": patch.value,
            "unset": patch.unset,
        })).collect::<Vec<_>>(),
        "daemon_revision": daemon_revision,
        "daemon_preview": daemon_preview,
    }))?;
    Ok(revision_for_bytes(&payload))
}

fn revision_for_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}
