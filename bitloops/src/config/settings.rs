//! Thin-CLI policy settings resolved from repo policy TOML plus global daemon CLI config.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue};

use super::{
    REPO_POLICY_FILE_NAME, REPO_POLICY_LOCAL_FILE_NAME, discover_repo_policy,
    discover_repo_policy_optional, load_daemon_settings,
};

pub const SETTINGS_DIR: &str = ".bitloops";
pub const SETTINGS_FILE: &str = REPO_POLICY_FILE_NAME;
pub const SETTINGS_LOCAL_FILE: &str = REPO_POLICY_LOCAL_FILE_NAME;
pub const DEFAULT_STRATEGY: &str = "manual-commit";

fn default_enabled() -> bool {
    true
}

fn default_strategy() -> String {
    DEFAULT_STRATEGY.to_string()
}

fn is_false(b: &bool) -> bool {
    !b
}

fn is_empty_str(s: &str) -> bool {
    s.is_empty()
}

fn is_empty_map(m: &HashMap<String, Value>) -> bool {
    m.is_empty()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BitloopsSettings {
    #[serde(default = "default_strategy")]
    pub strategy: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub local_dev: bool,
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub log_level: String,
    #[serde(default, skip_serializing_if = "is_empty_map")]
    pub strategy_options: HashMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<bool>,
}

impl Default for BitloopsSettings {
    fn default() -> Self {
        Self {
            strategy: DEFAULT_STRATEGY.to_string(),
            enabled: true,
            local_dev: false,
            log_level: String::new(),
            strategy_options: HashMap::new(),
            telemetry: None,
        }
    }
}

pub fn settings_path(repo_root: &Path) -> PathBuf {
    repo_root.join(SETTINGS_FILE)
}

pub fn settings_local_path(repo_root: &Path) -> PathBuf {
    repo_root.join(SETTINGS_LOCAL_FILE)
}

pub fn load_settings(repo_root: &Path) -> Result<BitloopsSettings> {
    load_settings_from_policy(discover_repo_policy_optional(repo_root)?)
}

pub fn load_required_settings(repo_root: &Path) -> Result<BitloopsSettings> {
    load_settings_from_policy(discover_repo_policy(repo_root)?)
}

fn load_settings_from_policy(policy: super::RepoPolicySnapshot) -> Result<BitloopsSettings> {
    let daemon_cli = daemon_cli_settings();

    let mut settings = BitloopsSettings {
        local_dev: daemon_cli.local_dev,
        log_level: daemon_cli.log_level,
        telemetry: daemon_cli.telemetry,
        ..BitloopsSettings::default()
    };

    if let Some(capture) = policy.capture.as_object() {
        if let Some(enabled) = capture.get("enabled").and_then(Value::as_bool) {
            settings.enabled = enabled;
        }
        if let Some(strategy) = capture
            .get("strategy")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            settings.strategy = strategy.to_string();
        }

        let mut strategy_options = capture.clone();
        strategy_options.remove("enabled");
        strategy_options.remove("strategy");
        settings.strategy_options = strategy_options.into_iter().collect();
    }

    Ok(settings)
}

#[cfg(test)]
fn daemon_cli_settings() -> super::daemon_config::DaemonCliSettings {
    if std::env::var_os(crate::test_support::process_state::SUPPRESS_HOST_DAEMON_CONFIG_ENV)
        .is_some()
    {
        super::daemon_config::DaemonCliSettings::default()
    } else {
        load_daemon_settings(None)
            .map(|loaded| loaded.cli)
            .unwrap_or_default()
    }
}

#[cfg(not(test))]
fn daemon_cli_settings() -> super::daemon_config::DaemonCliSettings {
    load_daemon_settings(None)
        .map(|loaded| loaded.cli)
        .unwrap_or_default()
}

pub fn current_config_fingerprint(repo_root: &Path) -> Result<String> {
    Ok(discover_repo_policy_optional(repo_root)?.fingerprint)
}

pub fn current_policy_root(repo_root: &Path) -> Result<Option<PathBuf>> {
    Ok(discover_repo_policy_optional(repo_root)?.root)
}

pub fn is_enabled(repo_root: &Path) -> Result<bool> {
    load_settings(repo_root).map(|settings| settings.enabled)
}

pub fn is_enabled_for_hooks(start: &Path) -> bool {
    discover_repo_policy_optional(start)
        .and_then(|policy| {
            let has_root = policy.root.is_some();
            load_settings_from_policy(policy).map(|settings| has_root && settings.enabled)
        })
        .unwrap_or(false)
}

pub fn save_settings(settings: &BitloopsSettings, path: &Path) -> Result<()> {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == SETTINGS_FILE || name == SETTINGS_LOCAL_FILE)
    {
        return save_repo_policy_settings(settings, path);
    }
    bail!(
        "unsupported settings target {}; repo policy settings must be written to `{}` or `{}`",
        path.display(),
        SETTINGS_FILE,
        SETTINGS_LOCAL_FILE
    );
}

impl BitloopsSettings {
    pub fn is_summarize_enabled(&self) -> bool {
        self.strategy_options
            .get("summarize")
            .and_then(Value::as_object)
            .and_then(|obj| obj.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn is_push_sessions_disabled(&self) -> bool {
        self.strategy_options
            .get("push_sessions")
            .and_then(Value::as_bool)
            .map(|enabled| !enabled)
            .unwrap_or(false)
    }
}

pub fn write_project_bootstrap_settings(
    path: &Path,
    strategy: &str,
    supported_agents: &[String],
) -> Result<()> {
    write_repo_policy_file(path, |doc| {
        ensure_capture_table(doc);
        doc["capture"]["enabled"] = Item::Value(TomlValue::from(true));
        doc["capture"]["strategy"] = Item::Value(TomlValue::from(strategy));
        ensure_agents_table(doc);
        doc["agents"]["supported"] = string_array_item(supported_agents);
        Ok(())
    })
}

pub fn set_scope_exclusions(
    path: &Path,
    exclude: &[String],
    exclude_from: &[String],
) -> Result<()> {
    write_repo_policy_file(path, |doc| {
        ensure_scope_table(doc);
        doc["scope"]["exclude"] = string_array_item(exclude);
        doc["scope"]["exclude_from"] = string_array_item(exclude_from);
        Ok(())
    })
}

pub fn set_capture_enabled(path: &Path, enabled: bool) -> Result<()> {
    write_repo_policy_file(path, |doc| {
        ensure_capture_table(doc);
        doc["capture"]["enabled"] = Item::Value(TomlValue::from(enabled));
        Ok(())
    })
}

fn save_repo_policy_settings(settings: &BitloopsSettings, path: &Path) -> Result<()> {
    write_repo_policy_file(path, |doc| {
        ensure_capture_table(doc);
        doc["capture"]["enabled"] = Item::Value(TomlValue::from(settings.enabled));
        doc["capture"]["strategy"] = Item::Value(TomlValue::from(settings.strategy.as_str()));
        for (key, value) in &settings.strategy_options {
            if value.is_null() {
                continue;
            }
            doc["capture"][key] = json_value_to_toml_item(value)?;
        }
        Ok(())
    })
}

fn json_value_to_toml_item(value: &Value) -> Result<Item> {
    match value {
        Value::Null => Ok(Item::None),
        Value::Bool(value) => Ok(Item::Value(TomlValue::from(*value))),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                return Ok(Item::Value(TomlValue::from(value)));
            }
            if let Some(value) = number.as_u64() {
                return Ok(Item::Value(TomlValue::from(value as i64)));
            }
            if let Some(value) = number.as_f64() {
                return Ok(Item::Value(TomlValue::from(value)));
            }
            bail!("unsupported numeric repo policy value `{number}`")
        }
        Value::String(value) => Ok(Item::Value(TomlValue::from(value.as_str()))),
        Value::Array(values) => {
            let mut array = Array::new();
            for value in values {
                array.push(match json_value_to_toml_item(value)? {
                    Item::Value(value) => value,
                    _ => bail!("repo policy arrays may only contain TOML scalar values"),
                });
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

fn write_repo_policy_file(
    path: &Path,
    update: impl FnOnce(&mut DocumentMut) -> Result<()>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("creating repo policy parent directory {}", parent.display())
        })?;
    }

    let existing = match fs::read_to_string(path) {
        Ok(existing) => existing,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(err).with_context(|| format!("reading repo policy {}", path.display()));
        }
    };
    let mut doc = if existing.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing repo policy {}", path.display()))?
    };

    update(&mut doc)?;

    fs::write(path, doc.to_string())
        .with_context(|| format!("writing repo policy {}", path.display()))
}

fn ensure_capture_table(doc: &mut DocumentMut) {
    if doc.get("capture").is_none_or(|item| !item.is_table()) {
        doc["capture"] = Item::Table(Table::new());
    }
}

fn ensure_agents_table(doc: &mut DocumentMut) {
    if doc.get("agents").is_none_or(|item| !item.is_table()) {
        doc["agents"] = Item::Table(Table::new());
    }
}

fn ensure_scope_table(doc: &mut DocumentMut) {
    if doc.get("scope").is_none_or(|item| !item.is_table()) {
        doc["scope"] = Item::Table(Table::new());
    }
}

fn string_array_item(values: &[String]) -> Item {
    let mut array = Array::new();
    for value in values {
        array.push(value.as_str());
    }
    Item::Value(TomlValue::Array(array))
}
