use anyhow::{Context, Result, bail};
use semver::Version;
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, de::from_str};

use crate::utils::platform_dirs::{bitloops_config_file_path, ensure_dir, ensure_parent_dir};

use super::resolve_blob_local_path_for_repo;
use super::unified_config::{UnifiedSettings, resolve_store_backend_from_unified};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DaemonCliSettings {
    pub local_dev: bool,
    pub cli_version: String,
    pub telemetry: Option<bool>,
    pub log_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonTelemetryConsentState {
    pub telemetry: Option<bool>,
    pub cli_version: String,
    pub needs_prompt: bool,
}

#[derive(Debug, Clone)]
pub struct LoadedDaemonSettings {
    pub path: PathBuf,
    pub root: PathBuf,
    pub settings: UnifiedSettings,
    pub cli: DaemonCliSettings,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
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
    semantic_clones: Option<Value>,
    #[serde(default)]
    embeddings: Option<Value>,
    #[serde(default)]
    dashboard: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RuntimeToml {
    #[serde(default)]
    local_dev: bool,
    cli_version: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TelemetryToml {
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct LoggingToml {
    level: Option<String>,
}

pub fn default_daemon_config_path() -> Result<PathBuf> {
    bitloops_config_file_path()
}

pub fn default_daemon_config_exists() -> Result<bool> {
    Ok(default_daemon_config_path()?.is_file())
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
        cli_version: file.runtime.cli_version.unwrap_or_default(),
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
            semantic_clones: file.semantic_clones,
            embeddings: file.embeddings,
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
    fs::write(&path, default_daemon_config_toml()?)
        .with_context(|| format!("writing Bitloops daemon config {}", path.display()))?;
    Ok(path)
}

pub fn bootstrap_default_daemon_environment() -> Result<PathBuf> {
    let path = ensure_daemon_config_exists()?;
    let loaded = load_daemon_settings(Some(path.as_path()))?;
    ensure_local_store_artifacts(&loaded)?;
    Ok(path)
}

pub fn persist_daemon_cli_settings(update: &DaemonCliSettings) -> Result<PathBuf> {
    persist_daemon_cli_settings_at(None, update)
}

pub fn update_daemon_telemetry_consent(
    explicit_path: Option<&Path>,
    current_cli_version: &str,
    telemetry_override: Option<bool>,
) -> Result<DaemonTelemetryConsentState> {
    let loaded = load_daemon_settings(explicit_path)?;
    let current = normalise_cli_version(current_cli_version)?;
    let mut cli = loaded.cli;

    if let Some(choice) = telemetry_override {
        cli.telemetry = Some(choice);
    } else if cli.telemetry == Some(false)
        && should_clear_telemetry_for_version(cli.cli_version.as_str(), &current)
    {
        cli.telemetry = None;
    }

    cli.cli_version = current;
    persist_daemon_cli_settings_at(Some(loaded.path.as_path()), &cli)?;

    Ok(DaemonTelemetryConsentState {
        telemetry: cli.telemetry,
        cli_version: cli.cli_version,
        needs_prompt: cli.telemetry.is_none(),
    })
}

fn persist_daemon_cli_settings_at(
    explicit_path: Option<&Path>,
    update: &DaemonCliSettings,
) -> Result<PathBuf> {
    let path = default_daemon_config_path()?;
    let path = explicit_path.map(Path::to_path_buf).unwrap_or(path);
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
        if update.cli_version.trim().is_empty() {
            runtime.remove("cli_version");
        } else {
            runtime["cli_version"] = Item::Value(update.cli_version.clone().into());
        }
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

    let dashboard = ensure_table(&mut doc, "dashboard");
    let local_dashboard = ensure_child_table(dashboard, "local_dashboard");
    local_dashboard["tls"] = Item::Value(enabled.into());

    fs::write(&path, doc.to_string())
        .with_context(|| format!("writing Bitloops daemon config {}", path.display()))?;
    Ok(path)
}

fn default_daemon_config_toml() -> Result<String> {
    let mut doc = DocumentMut::new();
    doc["runtime"] = Item::Table(Table::new());
    doc["runtime"]["local_dev"] = Item::Value(false.into());

    let default_root = Path::new(".");
    let sqlite_path = crate::utils::paths::default_relational_db_path(default_root);
    let duckdb_path = crate::utils::paths::default_events_db_path(default_root);
    let blob_path = crate::utils::paths::default_blob_store_path(default_root);

    doc["stores"] = Item::Table(Table::new());
    doc["stores"]["relational"] = Item::Table(Table::new());
    doc["stores"]["relational"]["sqlite_path"] =
        Item::Value(sqlite_path.to_string_lossy().to_string().into());
    doc["stores"]["events"] = Item::Table(Table::new());
    doc["stores"]["events"]["duckdb_path"] =
        Item::Value(duckdb_path.to_string_lossy().to_string().into());
    doc["stores"]["blob"] = Item::Table(Table::new());
    doc["stores"]["blob"]["local_path"] =
        Item::Value(blob_path.to_string_lossy().to_string().into());

    Ok(doc.to_string())
}

fn normalise_cli_version(current_cli_version: &str) -> Result<String> {
    let trimmed = current_cli_version.trim();
    if trimmed.is_empty() {
        bail!("current CLI version must not be empty");
    }
    Version::parse(trimmed).context("current CLI version must be valid semver")?;
    Ok(trimmed.to_string())
}

fn should_clear_telemetry_for_version(stored_cli_version: &str, current: &str) -> bool {
    let trimmed = stored_cli_version.trim();
    if trimmed.is_empty() {
        return true;
    }

    let Ok(stored) = Version::parse(trimmed) else {
        return true;
    };
    let Ok(current) = Version::parse(current) else {
        return false;
    };
    stored < current
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

fn ensure_child_table<'a>(table: &'a mut Table, key: &str) -> &'a mut Table {
    if !table.contains_key(key) || !table[key].is_table() {
        table.insert(key, Item::Table(Table::new()));
    }
    table[key]
        .as_table_mut()
        .expect("TOML item should be a table after initialisation")
}

fn ensure_local_store_artifacts(loaded: &LoadedDaemonSettings) -> Result<()> {
    let backends = resolve_store_backend_from_unified(&loaded.settings, &loaded.root)
        .with_context(|| format!("resolving store backends from {}", loaded.path.display()))?;

    if !backends.relational.has_postgres() {
        let sqlite_path = backends
            .relational
            .resolve_sqlite_db_path_for_repo(&loaded.root)
            .context("resolving SQLite path for daemon bootstrap")?;
        if let Some(parent) = sqlite_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating SQLite directory {}", parent.display()))?;
        }
        let _ = rusqlite::Connection::open(&sqlite_path)
            .with_context(|| format!("creating SQLite database at {}", sqlite_path.display()))?;
    }

    if !backends.events.has_clickhouse() {
        let duckdb_path = backends
            .events
            .resolve_duckdb_db_path_for_repo(&loaded.root);
        if let Some(parent) = duckdb_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating DuckDB directory {}", parent.display()))?;
        }
        let _ = duckdb::Connection::open(&duckdb_path)
            .with_context(|| format!("creating DuckDB database at {}", duckdb_path.display()))?;
    }

    if !backends.blobs.has_remote() {
        let blob_root =
            resolve_blob_local_path_for_repo(&loaded.root, backends.blobs.local_path.as_deref())
                .context("resolving blob store path for daemon bootstrap")?;
        fs::create_dir_all(&blob_root)
            .with_context(|| format!("creating blob store root {}", blob_root.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn load_daemon_settings_rejects_unknown_top_level_fields() {
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

        let err = load_daemon_settings(Some(config.path())).expect_err("unknown top-level key");
        let message = format!("{err:#}");
        assert!(
            message.contains("unknown field `cli_version`"),
            "expected unknown field error, got: {message}"
        );
    }

    #[test]
    fn load_daemon_settings_accepts_runtime_cli_version_field() {
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
