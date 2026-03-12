//! Shared DevQL config file parsing and path.
//! Used by both the CLI (devql commands) and the dashboard server so supported keys and defaults stay in sync.

use anyhow::{Result, bail};
use serde_json::{Map, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Relative path from user home to the DevQL config file (e.g. `~/.bitloops/config.json`).
pub const DEVQL_CONFIG_RELATIVE_PATH: &str = ".bitloops/config.json";
pub const DEVQL_DUCKDB_DEFAULT_PATH: &str = "~/.bitloops/devql/events.duckdb";
/// Default relative path from user home to the local SQLite relational DB file.
pub const DEVQL_SQLITE_RELATIVE_PATH: &str = ".bitloops/devql/relational.db";

const ENV_RELATIONAL_PROVIDER: &str = "BITLOOPS_DEVQL_RELATIONAL_PROVIDER";
const ENV_EVENTS_PROVIDER: &str = "BITLOOPS_DEVQL_EVENTS_PROVIDER";
const ENV_SQLITE_PATH: &str = "BITLOOPS_DEVQL_SQLITE_PATH";
const ENV_DUCKDB_PATH: &str = "BITLOOPS_DEVQL_DUCKDB_PATH";
const ENV_POSTGRES_DSN: &str = "BITLOOPS_DEVQL_PG_DSN";
const ENV_CLICKHOUSE_URL: &str = "BITLOOPS_DEVQL_CH_URL";
const ENV_CLICKHOUSE_USER: &str = "BITLOOPS_DEVQL_CH_USER";
const ENV_CLICKHOUSE_PASSWORD: &str = "BITLOOPS_DEVQL_CH_PASSWORD";
const ENV_CLICKHOUSE_DATABASE: &str = "BITLOOPS_DEVQL_CH_DATABASE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationalProvider {
    Sqlite,
    Postgres,
}

impl RelationalProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventsProvider {
    DuckDb,
    ClickHouse,
}

impl EventsProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DuckDb => "duckdb",
            Self::ClickHouse => "clickhouse",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevqlBackendConfig {
    pub relational: RelationalBackendConfig,
    pub events: EventsBackendConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationalBackendConfig {
    pub provider: RelationalProvider,
    pub sqlite_path: Option<String>,
    pub postgres_dsn: Option<String>,
}

impl RelationalBackendConfig {
    pub fn resolve_sqlite_db_path(&self) -> Result<PathBuf> {
        resolve_sqlite_db_path(self.sqlite_path.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsBackendConfig {
    pub provider: EventsProvider,
    pub duckdb_path: Option<String>,
    pub clickhouse_url: Option<String>,
    pub clickhouse_user: Option<String>,
    pub clickhouse_password: Option<String>,
    pub clickhouse_database: Option<String>,
}

impl EventsBackendConfig {
    pub fn clickhouse_endpoint(&self) -> String {
        let base = self
            .clickhouse_url
            .clone()
            .unwrap_or_else(|| "http://localhost:8123".to_string());
        let database = self
            .clickhouse_database
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let base = base.trim_end_matches('/');
        format!("{base}/?database={database}")
    }

    pub fn duckdb_path_or_default(&self) -> PathBuf {
        match self.duckdb_path.as_deref() {
            // For an explicitly configured path, preserve existing behavior:
            // expand a leading '~' if possible, otherwise return it as-is.
            Some(path) => expand_tilde_path(path),
            // For the default path, avoid creating a literal '~' directory when
            // HOME/USERPROFILE is not set by resolving the home directory
            // explicitly and falling back to a relative path without '~'.
            None => {
                // DEVQL_DUCKDB_DEFAULT_PATH is "~/.bitloops/devql/events.duckdb"
                let relative = DEVQL_DUCKDB_DEFAULT_PATH
                    .strip_prefix("~/")
                    .unwrap_or(DEVQL_DUCKDB_DEFAULT_PATH);

                if let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) {
                    PathBuf::from(home).join(relative)
                } else {
                    // Fall back to a relative path without '~' to avoid creating
                    // a directory literally named "~" in the current working directory.
                    PathBuf::from(relative)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DevqlFileConfig {
    pub(crate) relational_provider: Option<String>,
    pub(crate) sqlite_path: Option<String>,
    pub(crate) pg_dsn: Option<String>,
    pub(crate) events_provider: Option<String>,
    pub(crate) duckdb_path: Option<String>,
    pub(crate) clickhouse_url: Option<String>,
    pub(crate) clickhouse_user: Option<String>,
    pub(crate) clickhouse_password: Option<String>,
    pub(crate) clickhouse_database: Option<String>,
    pub(crate) _semantic_provider: Option<String>,
    pub(crate) _semantic_model: Option<String>,
    pub(crate) _semantic_api_key: Option<String>,
    pub(crate) _semantic_base_url: Option<String>,
}

impl DevqlFileConfig {
    /// Load config from `$HOME/.bitloops/config.json` (or `$USERPROFILE` on Windows).
    /// Returns default if the file is missing or invalid.
    pub fn load() -> Self {
        let Some(path) = user_home_config_path() else {
            return Self::default();
        };

        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(_) => return Self::default(),
        };
        let value: Value = match serde_json::from_slice(&data) {
            Ok(value) => value,
            Err(_) => return Self::default(),
        };
        Self::from_json_value(&value)
    }

    /// Parse config from a JSON value (e.g. from file or tests).
    /// Looks for a `"devql"` object first, then falls back to the root object.
    pub fn from_json_value(value: &Value) -> Self {
        let root_opt = value
            .get("devql")
            .and_then(Value::as_object)
            .or_else(|| value.as_object());
        let Some(root) = root_opt else {
            return Self::default();
        };

        let relational = root.get("relational").and_then(Value::as_object);
        let events = root.get("events").and_then(Value::as_object);

        Self {
            relational_provider: read_any_string_opt(
                relational,
                &["provider", "relational_provider", ENV_RELATIONAL_PROVIDER],
            )
            .or_else(|| read_any_string(root, &["relational_provider", ENV_RELATIONAL_PROVIDER])),
            sqlite_path: read_any_string_opt(relational, &["sqlite_path", "path"])
                .or_else(|| read_any_string(root, &["sqlite_path", ENV_SQLITE_PATH])),
            pg_dsn: read_any_string_opt(relational, &["postgres_dsn", "pg_dsn", ENV_POSTGRES_DSN])
                .or_else(|| read_any_string(root, &["postgres_dsn", "pg_dsn", ENV_POSTGRES_DSN])),
            events_provider: read_any_string_opt(events, &["provider", "events_provider"])
                .or_else(|| read_any_string(root, &["events_provider", ENV_EVENTS_PROVIDER])),
            duckdb_path: read_any_string_opt(events, &["duckdb_path", "path"])
                .or_else(|| read_any_string(root, &["duckdb_path", ENV_DUCKDB_PATH])),
            clickhouse_url: read_any_string_opt(events, &["clickhouse_url", "ch_url"]).or_else(
                || read_any_string(root, &["clickhouse_url", "ch_url", ENV_CLICKHOUSE_URL]),
            ),
            clickhouse_user: read_any_string_opt(events, &["clickhouse_user", "ch_user"]).or_else(
                || read_any_string(root, &["clickhouse_user", "ch_user", ENV_CLICKHOUSE_USER]),
            ),
            clickhouse_password: read_any_string_opt(
                events,
                &["clickhouse_password", "ch_password"],
            )
            .or_else(|| {
                read_any_string(
                    root,
                    &[
                        "clickhouse_password",
                        "ch_password",
                        ENV_CLICKHOUSE_PASSWORD,
                    ],
                )
            }),
            clickhouse_database: read_any_string_opt(
                events,
                &["clickhouse_database", "ch_database"],
            )
            .or_else(|| {
                read_any_string(
                    root,
                    &[
                        "clickhouse_database",
                        "ch_database",
                        ENV_CLICKHOUSE_DATABASE,
                    ],
                )
            }),
            _semantic_provider: read_any_string(
                root,
                &["semantic_provider", "BITLOOPS_DEVQL_SEMANTIC_PROVIDER"],
            ),
            _semantic_model: read_any_string(
                root,
                &["semantic_model", "BITLOOPS_DEVQL_SEMANTIC_MODEL"],
            ),
            _semantic_api_key: read_any_string(
                root,
                &["semantic_api_key", "BITLOOPS_DEVQL_SEMANTIC_API_KEY"],
            ),
            _semantic_base_url: read_any_string(
                root,
                &["semantic_base_url", "BITLOOPS_DEVQL_SEMANTIC_BASE_URL"],
            ),
        }
    }
}

pub fn resolve_devql_backend_config() -> Result<DevqlBackendConfig> {
    let file_cfg = DevqlFileConfig::load();
    resolve_devql_backend_config_with(file_cfg, |key| env::var(key).ok())
}

pub fn resolve_sqlite_db_path(raw_path: Option<&str>) -> Result<PathBuf> {
    match raw_path {
        Some(raw) if !raw.trim().is_empty() => normalize_sqlite_path(raw),
        _ => {
            let Some(home) = user_home_dir() else {
                bail!(
                    "unable to resolve home directory for default SQLite path; configure `devql.relational.sqlite_path` or `BITLOOPS_DEVQL_SQLITE_PATH`"
                );
            };
            Ok(home.join(DEVQL_SQLITE_RELATIVE_PATH))
        }
    }
}

fn resolve_devql_backend_config_with<F>(
    file_cfg: DevqlFileConfig,
    env_lookup: F,
) -> Result<DevqlBackendConfig>
where
    F: Fn(&str) -> Option<String>,
{
    let env_rel_provider = read_non_empty_env(&env_lookup, ENV_RELATIONAL_PROVIDER);
    let env_events_provider = read_non_empty_env(&env_lookup, ENV_EVENTS_PROVIDER);

    let sqlite_path = read_non_empty_env(&env_lookup, ENV_SQLITE_PATH).or(file_cfg.sqlite_path);
    let postgres_dsn = read_non_empty_env(&env_lookup, ENV_POSTGRES_DSN).or(file_cfg.pg_dsn);

    let duckdb_path = read_non_empty_env(&env_lookup, ENV_DUCKDB_PATH).or(file_cfg.duckdb_path);
    let clickhouse_url =
        read_non_empty_env(&env_lookup, ENV_CLICKHOUSE_URL).or(file_cfg.clickhouse_url);
    let clickhouse_user =
        read_non_empty_env(&env_lookup, ENV_CLICKHOUSE_USER).or(file_cfg.clickhouse_user);
    let clickhouse_password =
        read_non_empty_env(&env_lookup, ENV_CLICKHOUSE_PASSWORD).or(file_cfg.clickhouse_password);
    let clickhouse_database =
        read_non_empty_env(&env_lookup, ENV_CLICKHOUSE_DATABASE).or(file_cfg.clickhouse_database);

    let relational_provider = if let Some(raw) = env_rel_provider.or(file_cfg.relational_provider) {
        parse_relational_provider(&raw)?
    } else if postgres_dsn.is_some() {
        RelationalProvider::Postgres
    } else {
        RelationalProvider::Sqlite
    };

    let clickhouse_legacy_detected = clickhouse_url.is_some()
        || clickhouse_user.is_some()
        || clickhouse_password.is_some()
        || clickhouse_database.is_some();
    let events_provider = if let Some(raw) = env_events_provider.or(file_cfg.events_provider) {
        parse_events_provider(&raw)?
    } else if clickhouse_legacy_detected {
        EventsProvider::ClickHouse
    } else {
        EventsProvider::DuckDb
    };

    Ok(DevqlBackendConfig {
        relational: RelationalBackendConfig {
            provider: relational_provider,
            sqlite_path,
            postgres_dsn,
        },
        events: EventsBackendConfig {
            provider: events_provider,
            duckdb_path,
            clickhouse_url,
            clickhouse_user,
            clickhouse_password,
            clickhouse_database,
        },
    })
}

fn user_home_config_path() -> Option<PathBuf> {
    user_home_dir().map(|home| home.join(DEVQL_CONFIG_RELATIVE_PATH))
}

fn user_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn normalize_sqlite_path(raw_path: &str) -> Result<PathBuf> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        bail!(
            "sqlite path is empty; set `devql.relational.sqlite_path` or `BITLOOPS_DEVQL_SQLITE_PATH`"
        );
    }

    let expanded = expand_home_prefix(trimmed)?;
    Ok(Path::new(&expanded).to_path_buf())
}

fn expand_home_prefix(path: &str) -> Result<String> {
    let home = user_home_dir();
    expand_home_prefix_with(path, home.as_deref())
}

fn expand_home_prefix_with(path: &str, home: Option<&Path>) -> Result<String> {
    if path == "~" {
        let Some(home) = home else {
            bail!("unable to resolve home directory for `~` path");
        };
        return Ok(home.to_string_lossy().to_string());
    }

    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        let Some(home) = home else {
            bail!("unable to resolve home directory for `~` path");
        };
        return Ok(home.join(rest).to_string_lossy().to_string());
    }

    Ok(path.to_string())
}

fn read_any_string(root: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = root.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn read_any_string_opt(root: Option<&Map<String, Value>>, keys: &[&str]) -> Option<String> {
    root.and_then(|map| read_any_string(map, keys))
}

fn read_non_empty_env<F>(env_lookup: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    env_lookup(key).and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn expand_tilde_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(trimmed));
    }

    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
        && let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(rest);
    }

    PathBuf::from(trimmed)
}

fn parse_relational_provider(raw: &str) -> Result<RelationalProvider> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "sqlite" => Ok(RelationalProvider::Sqlite),
        "postgres" | "postgresql" => Ok(RelationalProvider::Postgres),
        other => {
            bail!("unsupported devql relational provider `{other}` (supported: sqlite, postgres)")
        }
    }
}

fn parse_events_provider(raw: &str) -> Result<EventsProvider> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "duckdb" => Ok(EventsProvider::DuckDb),
        "clickhouse" => Ok(EventsProvider::ClickHouse),
        other => {
            bail!("unsupported devql events provider `{other}` (supported: duckdb, clickhouse)")
        }
    }
}

#[cfg(test)]
pub(crate) fn resolve_devql_backend_config_for_tests(
    file_cfg: DevqlFileConfig,
    env: &[(&str, &str)],
) -> Result<DevqlBackendConfig> {
    resolve_devql_backend_config_with(file_cfg, |key| {
        env.iter().find_map(|(k, v)| {
            if *k == key {
                Some((*v).to_string())
            } else {
                None
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_config_defaults_to_sqlite_and_duckdb() {
        let cfg =
            resolve_devql_backend_config_for_tests(DevqlFileConfig::default(), &[]).expect("cfg");

        assert_eq!(cfg.relational.provider, RelationalProvider::Sqlite);
        assert_eq!(cfg.events.provider, EventsProvider::DuckDb);
    }

    #[test]
    fn backend_config_infers_legacy_postgres_clickhouse() {
        let value = serde_json::json!({
            "devql": {
                "postgres_dsn": "postgres://u:p@localhost:5432/bitloops",
                "clickhouse_url": "http://localhost:8123",
                "clickhouse_database": "bitloops"
            }
        });
        let file_cfg = DevqlFileConfig::from_json_value(&value);

        let cfg = resolve_devql_backend_config_for_tests(file_cfg, &[]).expect("cfg");
        assert_eq!(cfg.relational.provider, RelationalProvider::Postgres);
        assert_eq!(cfg.events.provider, EventsProvider::ClickHouse);
        assert_eq!(
            cfg.relational.postgres_dsn.as_deref(),
            Some("postgres://u:p@localhost:5432/bitloops")
        );
        assert_eq!(
            cfg.events.clickhouse_url.as_deref(),
            Some("http://localhost:8123")
        );
        assert_eq!(cfg.events.clickhouse_database.as_deref(), Some("bitloops"));
    }

    #[test]
    fn backend_config_honors_env_over_file_precedence() {
        let value = serde_json::json!({
            "devql": {
                "relational": {
                    "provider": "sqlite",
                    "sqlite_path": "/tmp/from-file.sqlite"
                },
                "events": {
                    "provider": "duckdb",
                    "duckdb_path": "/tmp/from-file.duckdb"
                },
                "postgres_dsn": "postgres://file-only",
                "clickhouse_url": "http://file-clickhouse:8123"
            }
        });
        let file_cfg = DevqlFileConfig::from_json_value(&value);
        let env = [
            (ENV_RELATIONAL_PROVIDER, "postgres"),
            (ENV_EVENTS_PROVIDER, "clickhouse"),
            (ENV_POSTGRES_DSN, "postgres://env-only"),
            (ENV_CLICKHOUSE_URL, "http://env-clickhouse:8123"),
            (ENV_CLICKHOUSE_DATABASE, "analytics"),
        ];

        let cfg = resolve_devql_backend_config_for_tests(file_cfg, &env).expect("cfg");
        assert_eq!(cfg.relational.provider, RelationalProvider::Postgres);
        assert_eq!(cfg.events.provider, EventsProvider::ClickHouse);
        assert_eq!(
            cfg.relational.postgres_dsn.as_deref(),
            Some("postgres://env-only")
        );
        assert_eq!(
            cfg.events.clickhouse_url.as_deref(),
            Some("http://env-clickhouse:8123")
        );
        assert_eq!(cfg.events.clickhouse_database.as_deref(), Some("analytics"));
    }

    #[test]
    fn backend_config_rejects_invalid_provider_values() {
        let env = [
            (ENV_RELATIONAL_PROVIDER, "mysql"),
            (ENV_EVENTS_PROVIDER, "kafka"),
        ];
        let err = resolve_devql_backend_config_for_tests(DevqlFileConfig::default(), &env)
            .expect_err("invalid provider must fail");

        let message = err.to_string();
        assert!(message.contains("unsupported devql"));
    }

    #[test]
    fn events_backend_duckdb_path_defaults_under_bitloops_directory() {
        let events = EventsBackendConfig {
            provider: EventsProvider::DuckDb,
            duckdb_path: None,
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        };

        let resolved = events.duckdb_path_or_default();
        let rendered = resolved.to_string_lossy();
        assert!(
            rendered.ends_with(".bitloops/devql/events.duckdb")
                || rendered.ends_with(".bitloops\\devql\\events.duckdb")
        );
    }

    #[test]
    fn events_backend_duckdb_path_preserves_explicit_path() {
        let events = EventsBackendConfig {
            provider: EventsProvider::DuckDb,
            duckdb_path: Some("/tmp/custom-events.duckdb".to_string()),
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        };

        assert_eq!(
            events.duckdb_path_or_default(),
            PathBuf::from("/tmp/custom-events.duckdb")
        );
    }

    #[test]
    fn sqlite_path_resolution_uses_explicit_path() {
        let resolved = resolve_sqlite_db_path(Some("/tmp/bitloops-relational.sqlite"))
            .expect("explicit sqlite path should resolve");
        assert_eq!(resolved, PathBuf::from("/tmp/bitloops-relational.sqlite"));
    }

    #[test]
    fn sqlite_path_resolution_expands_tilde_prefix() {
        let Some(home) = user_home_dir() else {
            return;
        };

        let resolved = resolve_sqlite_db_path(Some("~/devql.sqlite"))
            .expect("tilde sqlite path should resolve");
        assert_eq!(resolved, home.join("devql.sqlite"));
    }

    #[test]
    fn sqlite_path_resolution_expands_windows_tilde_prefix_with_windows_home() {
        let windows_home = Path::new(r"C:\Users\bitloops");

        let expanded =
            expand_home_prefix_with(r"~\.bitloops\devql\relational.db", Some(windows_home))
                .expect("windows-style tilde sqlite path should resolve");

        assert_eq!(
            PathBuf::from(expanded),
            windows_home.join(r".bitloops\devql\relational.db")
        );
    }
}
