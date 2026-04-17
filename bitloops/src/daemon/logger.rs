use anyhow::{Context, Result};
use chrono::{SecondsFormat, TimeZone, Utc};
use env_logger::Target;
use log::{LevelFilter, Record};
use serde::Serialize;
use serde_json::{Map, Value};
use std::env;
#[cfg(test)]
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::load_daemon_settings;

use super::log_file::open_daemon_log_sink;

pub const DAEMON_LOG_FILE_NAME: &str = "daemon.log";
pub const LOG_LEVEL_ENV_VAR: &str = "BITLOOPS_LOG_LEVEL";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProcessLogContext {
    process: &'static str,
    mode: &'static str,
    config_path: Option<PathBuf>,
    service_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedLogLevel {
    level: LevelFilter,
    invalid_value: Option<String>,
}

impl Default for ResolvedLogLevel {
    fn default() -> Self {
        Self {
            level: LevelFilter::Info,
            invalid_value: None,
        }
    }
}

impl ProcessLogContext {
    pub fn daemon(
        mode: &'static str,
        config_path: Option<PathBuf>,
        service_name: Option<String>,
    ) -> Self {
        Self {
            process: "daemon",
            mode,
            config_path,
            service_name,
        }
    }

    pub fn daemon_cli(mode: &'static str, config_path: Option<PathBuf>) -> Self {
        Self {
            process: "daemon_cli",
            mode,
            config_path,
            service_name: None,
        }
    }

    pub fn supervisor() -> Self {
        Self {
            process: "daemon_supervisor",
            mode: "supervisor",
            config_path: None,
            service_name: Some(super::GLOBAL_SUPERVISOR_SERVICE_NAME.to_string()),
        }
    }

    pub fn watcher(config_path: Option<PathBuf>) -> Self {
        Self {
            process: "watcher",
            mode: "watcher",
            config_path,
            service_name: None,
        }
    }
}

pub fn daemon_log_file_path() -> PathBuf {
    crate::utils::platform_dirs::bitloops_state_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("bitloops").join("state"))
        .join("logs")
        .join(DAEMON_LOG_FILE_NAME)
}

pub fn init_process_logger(context: ProcessLogContext, require_log_file: bool) -> Result<()> {
    let resolved_level = resolve_log_level(context.config_path.as_deref());
    if let Some(value) = resolved_level.invalid_value.as_ref() {
        eprintln!(
            "[bitloops] Warning: invalid daemon log level {:?}, defaulting to INFO",
            value
        );
    }

    let target = daemon_log_target(require_log_file)?;
    let format_context = context.clone();
    let mut logger = env_logger::Builder::new();
    logger.filter_level(resolved_level.level);
    logger.target(target);
    logger.format(move |buf, record| {
        let entry = build_log_entry(record, &format_context, current_time_millis());
        writeln!(buf, "{entry}")
    });
    logger
        .try_init()
        .context("initializing Bitloops daemon process logger")
}

fn daemon_log_target(require_log_file: bool) -> Result<Target> {
    let log_path = daemon_log_file_path();
    match open_daemon_log_sink(&log_path) {
        Ok(file) => Ok(Target::Pipe(Box::new(file))),
        Err(err) => {
            if require_log_file {
                return Err(err).with_context(|| {
                    format!("opening required daemon log file {}", log_path.display())
                });
            }
            eprintln!(
                "[bitloops] Warning: failed to open daemon log file {}: {err}",
                log_path.display()
            );
            Ok(Target::Stderr)
        }
    }
}

fn resolve_log_level(explicit_config_path: Option<&Path>) -> ResolvedLogLevel {
    if let Some(value) = env::var(LOG_LEVEL_ENV_VAR).ok()
        && !value.trim().is_empty()
    {
        return parse_log_level(&value).map_or(
            ResolvedLogLevel {
                level: LevelFilter::Info,
                invalid_value: Some(value),
            },
            |level| ResolvedLogLevel {
                level,
                invalid_value: None,
            },
        );
    }

    let config_value = load_daemon_settings(explicit_config_path)
        .ok()
        .map(|loaded| loaded.cli.log_level)
        .filter(|value| !value.trim().is_empty());
    if let Some(value) = config_value {
        return parse_log_level(&value).map_or(
            ResolvedLogLevel {
                level: LevelFilter::Info,
                invalid_value: Some(value),
            },
            |level| ResolvedLogLevel {
                level,
                invalid_value: None,
            },
        );
    }

    ResolvedLogLevel::default()
}

fn parse_log_level(value: &str) -> Option<LevelFilter> {
    match value.trim().to_ascii_uppercase().as_str() {
        "DEBUG" => Some(LevelFilter::Debug),
        "INFO" => Some(LevelFilter::Info),
        "WARN" | "WARNING" => Some(LevelFilter::Warn),
        "ERROR" => Some(LevelFilter::Error),
        _ => None,
    }
}

fn build_log_entry(record: &Record<'_>, context: &ProcessLogContext, timestamp_ms: u128) -> Value {
    let mut object = Map::new();
    object.insert(
        "time".to_string(),
        Value::String(format_timestamp(timestamp_ms)),
    );
    object.insert(
        "level".to_string(),
        Value::String(record.level().as_str().to_string()),
    );
    object.insert("msg".to_string(), Value::String(record.args().to_string()));
    object.insert(
        "target".to_string(),
        Value::String(record.target().to_string()),
    );
    object.insert(
        "module".to_string(),
        optional_string_value(record.module_path()),
    );
    object.insert("file".to_string(), optional_string_value(record.file()));
    object.insert(
        "line".to_string(),
        record.line().map(Value::from).unwrap_or(Value::Null),
    );
    object.insert("pid".to_string(), Value::from(std::process::id()));
    object.insert(
        "process".to_string(),
        Value::String(context.process.to_string()),
    );
    object.insert("mode".to_string(), Value::String(context.mode.to_string()));
    object.insert(
        "config_path".to_string(),
        context
            .config_path
            .as_ref()
            .map(|path| Value::String(path.display().to_string()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "service_name".to_string(),
        context
            .service_name
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    Value::Object(object)
}

fn optional_string_value(value: Option<&str>) -> Value {
    value
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null)
}

fn format_timestamp(timestamp_ms: u128) -> String {
    i64::try_from(timestamp_ms)
        .ok()
        .and_then(|timestamp_ms| Utc.timestamp_millis_opt(timestamp_ms).single())
        .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
        .unwrap_or_else(|| timestamp_ms.to_string())
}

fn current_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::process_state::enter_process_state;
    use log::{Level, Record};
    use tempfile::TempDir;

    fn write_daemon_config(config_root: &Path, content: &str) {
        let config_path = config_root.join("bitloops").join("config.toml");
        let parent = config_path.parent().expect("config parent");
        fs::create_dir_all(parent).expect("create daemon config parent");
        fs::write(config_path, content).expect("write daemon config");
    }

    #[test]
    fn daemon_log_file_path_uses_state_dir() {
        let state_root = TempDir::new().expect("temp dir");
        let state_root_str = state_root.path().to_string_lossy().to_string();
        let _guard = enter_process_state(
            None,
            &[(
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_str.as_str()),
            )],
        );

        assert_eq!(
            daemon_log_file_path(),
            state_root
                .path()
                .join("bitloops")
                .join("logs")
                .join(DAEMON_LOG_FILE_NAME)
        );
    }

    #[test]
    fn resolve_log_level_prefers_env_override() {
        let _guard = enter_process_state(None, &[(LOG_LEVEL_ENV_VAR, Some("DEBUG"))]);

        assert_eq!(
            resolve_log_level(None),
            ResolvedLogLevel {
                level: LevelFilter::Debug,
                invalid_value: None,
            }
        );
    }

    #[test]
    fn resolve_log_level_uses_config_fallback() {
        let config_root = TempDir::new().expect("temp dir");
        let config_root_str = config_root.path().to_string_lossy().to_string();
        write_daemon_config(config_root.path(), "[logging]\nlevel = \"WARN\"\n");
        let _guard = enter_process_state(
            None,
            &[
                (
                    "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                    Some(config_root_str.as_str()),
                ),
                (LOG_LEVEL_ENV_VAR, None),
            ],
        );

        assert_eq!(
            resolve_log_level(None),
            ResolvedLogLevel {
                level: LevelFilter::Warn,
                invalid_value: None,
            }
        );
    }

    #[test]
    fn resolve_log_level_defaults_to_info() {
        let _guard = enter_process_state(None, &[(LOG_LEVEL_ENV_VAR, None)]);

        assert_eq!(resolve_log_level(None).level, LevelFilter::Info);
    }

    #[test]
    fn resolve_log_level_invalid_value_falls_back_to_info() {
        let _guard = enter_process_state(None, &[(LOG_LEVEL_ENV_VAR, Some("LOUD"))]);

        assert_eq!(
            resolve_log_level(None),
            ResolvedLogLevel {
                level: LevelFilter::Info,
                invalid_value: Some("LOUD".to_string()),
            }
        );
    }

    #[test]
    fn build_log_entry_includes_expected_fields() {
        let context = ProcessLogContext::daemon(
            "service",
            Some(PathBuf::from("/tmp/bitloops/config.toml")),
            Some("com.bitloops.daemon".to_string()),
        );
        let message = format_args!("daemon ready");
        let record = Record::builder()
            .args(message)
            .level(Level::Info)
            .target("bitloops::daemon")
            .module_path_static(Some("bitloops::daemon::tests"))
            .file_static(Some("src/daemon/logger.rs"))
            .line(Some(42))
            .build();

        let entry = build_log_entry(&record, &context, 1234);

        assert_eq!(entry["level"], "INFO");
        assert_eq!(entry["msg"], "daemon ready");
        assert_eq!(entry["target"], "bitloops::daemon");
        assert_eq!(entry["module"], "bitloops::daemon::tests");
        assert_eq!(entry["file"], "src/daemon/logger.rs");
        assert_eq!(entry["line"], 42);
        assert!(entry["time"].is_string());
        assert_eq!(entry["process"], "daemon");
        assert_eq!(entry["mode"], "service");
        assert_eq!(entry["config_path"], "/tmp/bitloops/config.toml");
        assert_eq!(entry["service_name"], "com.bitloops.daemon");
        assert!(entry.get("pid").is_some());
    }

    #[test]
    fn build_log_entry_formats_time_as_rfc3339_utc() {
        let context = ProcessLogContext::daemon("service", None, None);
        let message = format_args!("daemon ready");
        let record = Record::builder()
            .args(message)
            .level(Level::Info)
            .target("bitloops::daemon")
            .build();

        let entry = build_log_entry(&record, &context, 1_234);
        let expected_time = Utc
            .timestamp_millis_opt(1_234)
            .single()
            .expect("valid UTC timestamp")
            .to_rfc3339_opts(SecondsFormat::Millis, true);

        assert_eq!(entry["time"], expected_time);
    }

    #[test]
    fn daemon_log_target_falls_back_to_stderr_when_file_logging_is_optional() {
        let temp = TempDir::new().expect("temp dir");
        let blocked_state_root = temp.path().join("blocked-state-root");
        fs::write(&blocked_state_root, "occupied").expect("write blocking state root file");
        let blocked_state_root_str = blocked_state_root.display().to_string();
        let _guard = enter_process_state(
            None,
            &[(
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(blocked_state_root_str.as_str()),
            )],
        );

        assert!(matches!(
            daemon_log_target(false).expect("daemon log target"),
            Target::Stderr
        ));
    }

    #[test]
    fn daemon_log_target_errors_when_file_logging_is_required() {
        let temp = TempDir::new().expect("temp dir");
        let blocked_state_root = temp.path().join("blocked-state-root");
        fs::write(&blocked_state_root, "occupied").expect("write blocking state root file");
        let blocked_state_root_str = blocked_state_root.display().to_string();
        let _guard = enter_process_state(
            None,
            &[(
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(blocked_state_root_str.as_str()),
            )],
        );

        let err = daemon_log_target(true).expect_err("required daemon log target should fail");
        assert!(err.to_string().contains(&blocked_state_root_str));
    }
}
