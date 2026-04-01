use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, de::from_str};

use crate::utils::platform_dirs::{bitloops_config_file_path, ensure_dir, ensure_parent_dir};

use super::unified_config::UnifiedSettings;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DaemonCliSettings {
    pub local_dev: bool,
    pub telemetry: Option<bool>,
    pub log_level: String,
}

#[derive(Debug, Clone)]
pub struct LoadedDaemonSettings {
    pub path: PathBuf,
    pub root: PathBuf,
    pub settings: UnifiedSettings,
    pub cli: DaemonCliSettings,
}

#[derive(Debug, Deserialize, Default)]
struct DaemonTomlFile {
    #[serde(default)]
    runtime: RuntimeToml,
    #[serde(default)]
    telemetry: TelemetryToml,
    #[serde(default)]
    logging: LoggingToml,
    #[serde(default)]
    stores: Option<Value>,
    #[serde(default)]
    knowledge: Option<Value>,
    #[serde(default)]
    semantic: Option<Value>,
    #[serde(default)]
    dashboard: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct RuntimeToml {
    #[serde(default)]
    local_dev: bool,
}

#[derive(Debug, Deserialize, Default)]
struct TelemetryToml {
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct LoggingToml {
    level: Option<String>,
}

pub fn default_daemon_config_path() -> Result<PathBuf> {
    bitloops_config_file_path()
}

pub fn load_daemon_settings(explicit_path: Option<&Path>) -> Result<LoadedDaemonSettings> {
    let path = match explicit_path {
        Some(path) => path.to_path_buf(),
        None => default_daemon_config_path()?,
    };
    let root = path
        .parent()
        .map(Path::to_path_buf)
        .context("resolving Bitloops daemon config directory")?;

    let file = match fs::read_to_string(&path) {
        Ok(data) => parse_daemon_config_text(&data, &path)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && explicit_path.is_none() => {
            DaemonTomlFile::default()
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            bail!("Bitloops daemon config not found at {}", path.display());
        }
        Err(err) => {
            return Err(err)
                .with_context(|| format!("reading Bitloops daemon config {}", path.display()));
        }
    };

    let cli = DaemonCliSettings {
        local_dev: file.runtime.local_dev,
        telemetry: file.telemetry.enabled,
        log_level: file.logging.level.unwrap_or_default(),
    };

    Ok(LoadedDaemonSettings {
        path,
        root,
        settings: UnifiedSettings {
            enabled: None,
            strategy: None,
            local_dev: Some(cli.local_dev),
            log_level: (!cli.log_level.is_empty()).then(|| cli.log_level.clone()),
            strategy_options: None,
            telemetry: cli.telemetry,
            stores: file.stores,
            knowledge: file.knowledge,
            semantic: file.semantic,
            dashboard: file.dashboard,
            watch: None,
        },
        cli,
    })
}

fn parse_daemon_config_text(data: &str, path: &Path) -> Result<DaemonTomlFile> {
    from_str::<DaemonTomlFile>(data)
        .with_context(|| format!("parsing Bitloops daemon config {}", path.display()))
}

pub fn ensure_daemon_config_exists() -> Result<PathBuf> {
    let path = default_daemon_config_path()?;
    if path.exists() {
        return Ok(path);
    }

    ensure_parent_dir(&path)?;
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    fs::write(&path, default_daemon_config_toml())
        .with_context(|| format!("writing Bitloops daemon config {}", path.display()))?;
    Ok(path)
}

pub fn persist_daemon_cli_settings(update: &DaemonCliSettings) -> Result<PathBuf> {
    let path = default_daemon_config_path()?;
    ensure_parent_dir(&path)?;

    let mut doc = match fs::read_to_string(&path) {
        Ok(existing) => existing
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing Bitloops daemon config {}", path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("reading Bitloops daemon config {}", path.display()));
        }
    };

    {
        let runtime = ensure_table(&mut doc, "runtime");
        runtime["local_dev"] = Item::Value(update.local_dev.into());
    }

    {
        let logging = ensure_table(&mut doc, "logging");
        if update.log_level.trim().is_empty() {
            logging.remove("level");
        } else {
            logging["level"] = Item::Value(update.log_level.clone().into());
        }
    }

    {
        let telemetry = ensure_table(&mut doc, "telemetry");
        match update.telemetry {
            Some(choice) => telemetry["enabled"] = Item::Value(choice.into()),
            None => {
                telemetry.remove("enabled");
            }
        }
    }

    fs::write(&path, doc.to_string())
        .with_context(|| format!("writing Bitloops daemon config {}", path.display()))?;
    Ok(path)
}

pub fn persist_dashboard_tls_hint(enabled: bool) -> Result<PathBuf> {
    let path = default_daemon_config_path()?;
    ensure_parent_dir(&path)?;

    let mut doc = match fs::read_to_string(&path) {
        Ok(existing) => existing
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing Bitloops daemon config {}", path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("reading Bitloops daemon config {}", path.display()));
        }
    };

    if !doc["dashboard"].is_table() {
        doc["dashboard"] = Item::Table(Table::new());
    }
    let dashboard = doc["dashboard"]
        .as_table_mut()
        .expect("dashboard should be a table");
    if !dashboard["local_dashboard"].is_table() {
        dashboard["local_dashboard"] = Item::Table(Table::new());
    }
    dashboard["local_dashboard"]["tls"] = Item::Value(enabled.into());

    fs::write(&path, doc.to_string())
        .with_context(|| format!("writing Bitloops daemon config {}", path.display()))?;
    Ok(path)
}

fn default_daemon_config_toml() -> String {
    let mut doc = DocumentMut::new();
    doc["runtime"] = Item::Table(Table::new());
    doc["runtime"]["local_dev"] = Item::Value(false.into());
    doc.to_string()
}

fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
    let root = doc.as_table_mut();
    if !root.contains_key(key) || !root[key].is_table() {
        root.insert(key, Item::Table(Table::new()));
    }
    root[key]
        .as_table_mut()
        .expect("TOML item should be a table after initialisation")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn load_daemon_settings_ignores_legacy_top_level_fields() {
        let config = NamedTempFile::new().expect("create temp config");
        fs::write(
            config.path(),
            r#"
cli_version = "0.0.3"

[runtime]
local_dev = true

[telemetry]
enabled = false

[logging]
level = "debug"
"#,
        )
        .expect("write temp config");

        let loaded = load_daemon_settings(Some(config.path())).expect("load daemon settings");
        assert!(loaded.cli.local_dev, "runtime.local_dev should be parsed");
        assert_eq!(loaded.cli.telemetry, Some(false));
        assert_eq!(loaded.cli.log_level, "debug");
    }

    #[test]
    fn load_daemon_settings_ignores_legacy_runtime_fields() {
        let config = NamedTempFile::new().expect("create temp config");
        fs::write(
            config.path(),
            r#"
[runtime]
local_dev = true
cli_version = "0.0.12"

[telemetry]
enabled = true

[logging]
level = "info"
"#,
        )
        .expect("write temp config");

        let loaded = load_daemon_settings(Some(config.path())).expect("load daemon settings");
        assert!(loaded.cli.local_dev, "runtime.local_dev should be parsed");
        assert_eq!(loaded.cli.telemetry, Some(true));
        assert_eq!(loaded.cli.log_level, "info");
    }
}
