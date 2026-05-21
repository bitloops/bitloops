use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, Result, bail};
use semver::Version;
use serde::Deserialize;
use serde_json::Value;
#[cfg(test)]
use toml_edit::Table;
use toml_edit::{Array, DocumentMut, Item, Value as TomlValue, de::from_str};

use crate::host::inference::{
    BITLOOPS_EMBEDDINGS_IPC_DRIVER, BITLOOPS_INFERENCE_RUNTIME_ID, BITLOOPS_PLATFORM_CHAT_DRIVER,
    BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID, CODEX_EXEC_DRIVER,
};
use crate::utils::platform_dirs::{
    bitloops_config_file_path, bitloops_data_dir, ensure_dir, ensure_parent_dir,
};

use super::super::resolve_blob_local_path_for_repo;
use super::super::unified_config::{UnifiedSettings, resolve_store_backend_from_unified};
use super::toml::{ensure_child_table, ensure_table};

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
    semantic_clones: Option<Value>,
    #[serde(default)]
    context_guidance: Option<Value>,
    #[serde(default)]
    architecture: Option<Value>,
    #[serde(default)]
    inference: Option<Value>,
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
            bail!(
                "Bitloops daemon config not found at {}. Run `bitloops start --create-default-config` or `bitloops init --install-default-daemon`.",
                path.display()
            );
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

    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
    let canonical_root = canonical_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(root.clone());

    Ok(LoadedDaemonSettings {
        path: canonical_path,
        root: canonical_root,
        settings: UnifiedSettings {
            enabled: None,
            strategy: None,
            local_dev: Some(cli.local_dev),
            log_level: (!cli.log_level.is_empty()).then(|| cli.log_level.clone()),
            strategy_options: None,
            telemetry: cli.telemetry,
            stores: file.stores,
            knowledge: file.knowledge,
            semantic_clones: file.semantic_clones,
            context_guidance: file.context_guidance,
            architecture: file.architecture,
            inference: file.inference,
            dashboard: file.dashboard,
            watch: None,
        },
        cli,
    })
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
    ensure_daemon_store_artifacts(Some(path.as_path()))?;
    Ok(path)
}

pub fn ensure_daemon_store_artifacts(explicit_path: Option<&Path>) -> Result<PathBuf> {
    let loaded = load_daemon_settings(explicit_path)?;
    ensure_local_store_artifacts(&loaded)?;
    Ok(loaded.path)
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

#[cfg(test)]
pub(crate) fn persist_daemon_store_backend_selection(
    source_path: &Path,
    target_path: &Path,
) -> Result<PathBuf> {
    let source = load_daemon_settings(Some(source_path))
        .with_context(|| format!("loading source daemon config {}", source_path.display()))?;
    ensure_parent_dir(target_path)?;

    let mut doc = match fs::read_to_string(target_path) {
        Ok(existing) => existing
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing Bitloops daemon config {}", target_path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(err) => {
            return Err(err).with_context(|| {
                format!("reading Bitloops daemon config {}", target_path.display())
            });
        }
    };

    let stores_root = source.settings.stores.as_ref().and_then(Value::as_object);
    let relational_root = stores_root
        .and_then(|stores| stores.get("relational"))
        .and_then(Value::as_object);
    let events_root = stores_root
        .and_then(|stores| stores.get("events").or_else(|| stores.get("event")))
        .and_then(Value::as_object);
    let blob_root = stores_root
        .and_then(|stores| stores.get("blobs").or_else(|| stores.get("blob")))
        .and_then(Value::as_object);

    let stores = ensure_table(&mut doc, "stores");

    {
        let relational = ensure_child_table(stores, "relational");
        set_or_remove_toml_string(
            relational,
            "postgres_dsn",
            relational_root
                .and_then(|root| root.get("postgres_dsn").or_else(|| root.get("pg_dsn")))
                .and_then(Value::as_str),
        );
    }

    {
        let events = ensure_child_table(stores, "events");
        set_or_remove_toml_string(
            events,
            "clickhouse_url",
            events_root
                .and_then(|root| root.get("clickhouse_url"))
                .and_then(Value::as_str),
        );
        set_or_remove_toml_string(
            events,
            "clickhouse_user",
            events_root
                .and_then(|root| root.get("clickhouse_user"))
                .and_then(Value::as_str),
        );
        set_or_remove_toml_string(
            events,
            "clickhouse_password",
            events_root
                .and_then(|root| root.get("clickhouse_password"))
                .and_then(Value::as_str),
        );
        set_or_remove_toml_string(
            events,
            "clickhouse_database",
            events_root
                .and_then(|root| root.get("clickhouse_database"))
                .and_then(Value::as_str),
        );
    }

    {
        let blobs = ensure_child_table(stores, "blob");
        set_or_remove_toml_string(
            blobs,
            "s3_bucket",
            blob_root
                .and_then(|root| root.get("s3_bucket"))
                .and_then(Value::as_str),
        );
        set_or_remove_toml_string(
            blobs,
            "s3_region",
            blob_root
                .and_then(|root| root.get("s3_region"))
                .and_then(Value::as_str),
        );
        set_or_remove_toml_string(
            blobs,
            "s3_access_key_id",
            blob_root
                .and_then(|root| root.get("s3_access_key_id"))
                .and_then(Value::as_str),
        );
        set_or_remove_toml_string(
            blobs,
            "s3_secret_access_key",
            blob_root
                .and_then(|root| root.get("s3_secret_access_key"))
                .and_then(Value::as_str),
        );
        set_or_remove_toml_string(
            blobs,
            "gcs_bucket",
            blob_root
                .and_then(|root| root.get("gcs_bucket"))
                .and_then(Value::as_str),
        );
        set_or_remove_toml_string(
            blobs,
            "gcs_credentials_path",
            blob_root
                .and_then(|root| root.get("gcs_credentials_path"))
                .and_then(Value::as_str),
        );
    }

    fs::write(target_path, doc.to_string())
        .with_context(|| format!("writing Bitloops daemon config {}", target_path.display()))?;
    Ok(target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.to_path_buf()))
}

fn parse_daemon_config_text(data: &str, path: &Path) -> Result<DaemonTomlFile> {
    from_str::<DaemonTomlFile>(data)
        .with_context(|| format!("parsing Bitloops daemon config {}", path.display()))
}

pub(crate) fn validate_daemon_config_text(data: &str, path: &Path) -> Result<()> {
    parse_daemon_config_text(data, path).map(|_| ())
}

#[cfg(test)]
fn set_or_remove_toml_string(table: &mut Table, key: &str, value: Option<&str>) {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => {
            table[key] = Item::Value(value.into());
        }
        None => {
            table.remove(key);
        }
    }
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

pub(crate) fn default_daemon_config_toml() -> Result<String> {
    let mut doc = DocumentMut::new();
    {
        let runtime = ensure_table(&mut doc, "runtime");
        runtime["local_dev"] = Item::Value(false.into());
        runtime["cli_version"] = Item::Value(env!("CARGO_PKG_VERSION").into());
    }

    let default_root = Path::new(".");
    let sqlite_path = crate::utils::paths::default_relational_db_path(default_root);
    let duckdb_path = crate::utils::paths::default_events_db_path(default_root);
    let blob_path = crate::utils::paths::default_blob_store_path(default_root);

    {
        let stores = ensure_table(&mut doc, "stores");
        let relational = ensure_child_table(stores, "relational");
        relational["sqlite_path"] = Item::Value(path_string(sqlite_path).into());

        let events = ensure_child_table(stores, "events");
        events["duckdb_path"] = Item::Value(path_string(duckdb_path).into());

        let blob = ensure_child_table(stores, "blob");
        blob["local_path"] = Item::Value(path_string(blob_path).into());
    }

    ensure_table(&mut doc, "logging");

    {
        let telemetry = ensure_table(&mut doc, "telemetry");
        telemetry["enabled"] = Item::Value(true.into());
    }

    let platform_embeddings_binary = default_managed_binary_path(
        "bitloops-platform-embeddings",
        managed_binary_name("bitloops-platform-embeddings"),
    );
    let bitloops_inference_binary = default_managed_binary_path(
        "bitloops-inference",
        managed_binary_name("bitloops-inference"),
    );
    let (codex_command, codex_args) = default_codex_runtime_command();

    {
        let inference = ensure_table(&mut doc, "inference");
        let runtimes = ensure_child_table(inference, "runtimes");

        let platform_embeddings =
            ensure_child_table(runtimes, BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID);
        platform_embeddings["command"] =
            Item::Value(path_string(platform_embeddings_binary).into());
        platform_embeddings["args"] = Item::Value(toml_string_array([
            "--api-key-env".to_string(),
            "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
        ]));
        platform_embeddings["startup_timeout_secs"] = Item::Value(60.into());
        platform_embeddings["request_timeout_secs"] = Item::Value(300.into());

        let bitloops_inference = ensure_child_table(runtimes, BITLOOPS_INFERENCE_RUNTIME_ID);
        bitloops_inference["command"] = Item::Value(path_string(bitloops_inference_binary).into());
        bitloops_inference["args"] = Item::Value(toml_string_array([]));
        bitloops_inference["startup_timeout_secs"] = Item::Value(60.into());
        bitloops_inference["request_timeout_secs"] = Item::Value(900.into());

        let codex = ensure_child_table(runtimes, "codex");
        codex["command"] = Item::Value(codex_command.into());
        codex["args"] = Item::Value(toml_string_array(codex_args));
        codex["startup_timeout_secs"] = Item::Value(5.into());
        codex["request_timeout_secs"] = Item::Value(600.into());

        let profiles = ensure_child_table(inference, "profiles");

        let platform_code = ensure_child_table(profiles, "platform_code");
        platform_code["task"] = Item::Value("embeddings".into());
        platform_code["driver"] = Item::Value(BITLOOPS_EMBEDDINGS_IPC_DRIVER.into());
        platform_code["runtime"] = Item::Value(BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID.into());
        platform_code["model"] = Item::Value("bge-m3".into());

        let guidance_llm = ensure_child_table(profiles, "guidance_llm");
        guidance_llm["task"] = Item::Value("text_generation".into());
        guidance_llm["runtime"] = Item::Value(BITLOOPS_INFERENCE_RUNTIME_ID.into());
        guidance_llm["driver"] = Item::Value(BITLOOPS_PLATFORM_CHAT_DRIVER.into());
        guidance_llm["model"] = Item::Value("ministral-3-3b-instruct".into());
        guidance_llm["api_key"] = Item::Value("${BITLOOPS_PLATFORM_GATEWAY_TOKEN}".into());
        guidance_llm["temperature"] = Item::Value("0.1".into());
        guidance_llm["max_output_tokens"] = Item::Value(4096.into());

        let summary_llm = ensure_child_table(profiles, "summary_llm");
        summary_llm["task"] = Item::Value("text_generation".into());
        summary_llm["runtime"] = Item::Value(BITLOOPS_INFERENCE_RUNTIME_ID.into());
        summary_llm["driver"] = Item::Value(BITLOOPS_PLATFORM_CHAT_DRIVER.into());
        summary_llm["model"] = Item::Value("ministral-3-3b-instruct".into());
        summary_llm["api_key"] = Item::Value("${BITLOOPS_PLATFORM_GATEWAY_TOKEN}".into());
        summary_llm["temperature"] = Item::Value("0.1".into());
        summary_llm["max_output_tokens"] = Item::Value(200.into());

        let fact_synthesis = ensure_child_table(profiles, "architecture_fact_synthesis_codex");
        fact_synthesis["task"] = Item::Value("structured_generation".into());
        fact_synthesis["runtime"] = Item::Value("codex".into());
        fact_synthesis["driver"] = Item::Value(CODEX_EXEC_DRIVER.into());
        fact_synthesis["model"] = Item::Value("gpt-5.4-mini".into());
        fact_synthesis["temperature"] = Item::Value("0.1".into());
        fact_synthesis["max_output_tokens"] = Item::Value(4096.into());
        fact_synthesis["thinking_level"] = Item::Value("low".into());

        let role_adjudication =
            ensure_child_table(profiles, "architecture_role_adjudication_codex");
        role_adjudication["task"] = Item::Value("structured_generation".into());
        role_adjudication["runtime"] = Item::Value("codex".into());
        role_adjudication["driver"] = Item::Value(CODEX_EXEC_DRIVER.into());
        role_adjudication["model"] = Item::Value("gpt-5.4-mini".into());
        role_adjudication["temperature"] = Item::Value("0.1".into());
        role_adjudication["max_output_tokens"] = Item::Value(1024.into());
        role_adjudication["thinking_level"] = Item::Value("low".into());
    }

    {
        let context_guidance = ensure_table(&mut doc, "context_guidance");
        let inference = ensure_child_table(context_guidance, "inference");
        inference["guidance_generation"] = Item::Value("guidance_llm".into());
    }

    {
        let semantic_clones = ensure_table(&mut doc, "semantic_clones");
        semantic_clones["summary_mode"] = Item::Value("auto".into());
        let inference = ensure_child_table(semantic_clones, "inference");
        inference["summary_generation"] = Item::Value("summary_llm".into());
    }

    {
        let architecture = ensure_table(&mut doc, "architecture");
        let inference = ensure_child_table(architecture, "inference");
        inference["fact_synthesis"] = Item::Value("architecture_fact_synthesis_codex".into());
        inference["role_adjudication"] = Item::Value("architecture_role_adjudication_codex".into());
    }

    Ok(doc.to_string())
}

fn path_string(path: PathBuf) -> String {
    path.to_string_lossy().to_string()
}

fn default_managed_binary_path(install_dir_name: &str, binary_name: String) -> PathBuf {
    default_data_dir()
        .join("tools")
        .join(install_dir_name)
        .join(binary_name)
}

fn default_data_dir() -> PathBuf {
    bitloops_data_dir().unwrap_or_else(|_| env::temp_dir().join("bitloops").join("data"))
}

fn managed_binary_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

fn toml_string_array(values: impl IntoIterator<Item = String>) -> TomlValue {
    let mut array = Array::new();
    for value in values {
        array.push(value);
    }
    TomlValue::Array(array)
}

fn default_codex_runtime_command() -> (String, Vec<String>) {
    let node = find_executable_on_path("node").unwrap_or_else(|| PathBuf::from("node"));
    let codex = find_executable_on_path("codex").unwrap_or_else(|| PathBuf::from("codex"));
    (
        path_string(node),
        vec![
            path_string(codex),
            "--ask-for-approval".to_string(),
            "never".to_string(),
        ],
    )
}

fn find_executable_on_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        for candidate in executable_candidates(&dir, name) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn executable_candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let candidate = dir.join(name);
        if candidate.extension().is_some() {
            return vec![candidate];
        }
        let pathext = env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter(|ext| !ext.trim().is_empty())
                    .map(|ext| dir.join(format!("{name}{ext}")))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![dir.join(format!("{name}.exe"))]);
        pathext
    }

    #[cfg(not(windows))]
    {
        vec![dir.join(name)]
    }
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

fn ensure_local_store_artifacts(loaded: &LoadedDaemonSettings) -> Result<()> {
    let backends = resolve_store_backend_from_unified(&loaded.settings, &loaded.root)
        .with_context(|| format!("resolving store backends from {}", loaded.path.display()))?;

    if !backends.relational.has_postgres() {
        let sqlite_path = backends
            .relational
            .resolve_sqlite_db_path_for_repo(&loaded.root)
            .context("resolving SQLite path for daemon bootstrap")?;
        if !sqlite_path.is_file() {
            if let Some(parent) = sqlite_path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating SQLite directory {}", parent.display()))?;
            }
            let _ = rusqlite::Connection::open(&sqlite_path).with_context(|| {
                format!("creating SQLite database at {}", sqlite_path.display())
            })?;
        }
    }

    if !backends.events.has_clickhouse() {
        let duckdb_path = backends
            .events
            .resolve_duckdb_db_path_for_repo(&loaded.root);
        if !duckdb_path.is_file() {
            if let Some(parent) = duckdb_path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating DuckDB directory {}", parent.display()))?;
            }
            let _ = duckdb::Connection::open(&duckdb_path).with_context(|| {
                format!("creating DuckDB database at {}", duckdb_path.display())
            })?;
        }
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
