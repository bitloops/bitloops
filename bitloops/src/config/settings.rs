//! Bitloops project settings (.bitloops/settings.json).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SETTINGS_DIR: &str = ".bitloops";
pub const SETTINGS_FILE: &str = "settings.json";
pub const SETTINGS_LOCAL_FILE: &str = "settings.local.json";
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

/// Project settings stored in `.bitloops/settings.json`.
///
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BitloopsSettings {
    /// Active session strategy. Defaults to "manual-commit".
    #[serde(default = "default_strategy")]
    pub strategy: String,

    /// Whether Bitloops is active. Defaults to true when field is absent.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Use local dev binary for hooks (development only).
    #[serde(default, skip_serializing_if = "is_false")]
    pub local_dev: bool,

    /// Logging verbosity.
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub log_level: String,

    /// Strategy-specific configuration.
    #[serde(default, skip_serializing_if = "is_empty_map")]
    pub strategy_options: HashMap<String, Value>,

    /// Telemetry opt-in state. `None` means not asked yet.
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

/// Returns path to `.bitloops/settings.json` under `repo_root`.
pub fn settings_path(repo_root: &Path) -> PathBuf {
    repo_root.join(SETTINGS_DIR).join(SETTINGS_FILE)
}

/// Returns path to `.bitloops/settings.local.json` under `repo_root`.
pub fn settings_local_path(repo_root: &Path) -> PathBuf {
    repo_root.join(SETTINGS_DIR).join(SETTINGS_LOCAL_FILE)
}

/// Loads merged settings: project file + local overrides.
///
pub fn load_settings(repo_root: &Path) -> Result<BitloopsSettings> {
    let base_path = settings_path(repo_root);
    let mut settings = load_from_file(&base_path)?;

    let local_path = settings_local_path(repo_root);
    match fs::read(&local_path) {
        Ok(data) => {
            merge_json(&mut settings, &data).context("merging local settings")?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(e).context("reading local settings file");
        }
    }

    apply_defaults(&mut settings);
    Ok(settings)
}

/// Loads settings from a single file path without local overrides.
///
fn load_from_file(path: &Path) -> Result<BitloopsSettings> {
    match fs::read(path) {
        Ok(data) => {
            let settings: BitloopsSettings = serde_json::from_slice(&data)
                .with_context(|| format!("parsing settings file: {}", path.display()))?;
            Ok(settings)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BitloopsSettings::default()),
        Err(e) => Err(e).with_context(|| format!("reading settings file: {}", path.display())),
    }
}

/// Merges JSON data into existing settings field-by-field.
/// Only non-zero / non-empty values from JSON override existing settings.
///
fn merge_json(settings: &mut BitloopsSettings, data: &[u8]) -> Result<()> {
    // Validate unknown keys first
    let _: BitloopsSettings = serde_json::from_slice(data).context("parsing JSON")?;

    // Parse as raw map to know which keys are present
    let raw: HashMap<String, Value> = serde_json::from_slice(data).context("parsing JSON")?;

    if let Some(v) = raw.get("strategy")
        && let Some(s) = v.as_str()
        && !s.is_empty()
    {
        settings.strategy = s.to_string();
    }

    if let Some(v) = raw.get("enabled")
        && let Some(b) = v.as_bool()
    {
        settings.enabled = b;
    }

    if let Some(v) = raw.get("local_dev")
        && let Some(b) = v.as_bool()
    {
        settings.local_dev = b;
    }

    if let Some(v) = raw.get("log_level")
        && let Some(s) = v.as_str()
        && !s.is_empty()
    {
        settings.log_level = s.to_string();
    }

    if let Some(v) = raw.get("strategy_options")
        && let Some(opts) = v.as_object()
    {
        for (k, val) in opts {
            settings.strategy_options.insert(k.clone(), val.clone());
        }
    }

    if let Some(v) = raw.get("telemetry")
        && let Some(b) = v.as_bool()
    {
        settings.telemetry = Some(b);
    }

    Ok(())
}

impl BitloopsSettings {
    /// Returns true when strategy option `summarize.enabled` is explicitly true.
    pub fn is_summarize_enabled(&self) -> bool {
        self.strategy_options
            .get("summarize")
            .and_then(Value::as_object)
            .and_then(|obj| obj.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// Returns true when strategy option `push_sessions` is explicitly false.
    pub fn is_push_sessions_disabled(&self) -> bool {
        self.strategy_options
            .get("push_sessions")
            .and_then(Value::as_bool)
            .map(|enabled| !enabled)
            .unwrap_or(false)
    }
}

fn apply_defaults(settings: &mut BitloopsSettings) {
    if settings.strategy.is_empty() {
        settings.strategy = DEFAULT_STRATEGY.to_string();
    }
}

/// Saves settings to the given path (creates parent directories as needed).
///
pub fn save_settings(settings: &BitloopsSettings, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating settings directory: {}", parent.display()))?;
    }
    let mut data = serde_json::to_string_pretty(settings).context("serializing settings")?;
    data.push('\n');
    fs::write(path, data).with_context(|| format!("writing settings file: {}", path.display()))?;
    Ok(())
}

/// Returns true when Bitloops is enabled (defaults to true if no settings file).
///
pub fn is_enabled(repo_root: &Path) -> Result<bool> {
    load_settings(repo_root).map(|s| s.enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create a temp dir and pre-create the .bitloops/ subdirectory.
    fn setup() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(SETTINGS_DIR)).unwrap();
        dir
    }

    fn write_settings(dir: &TempDir, content: &str) {
        let path = settings_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn write_local_settings(dir: &TempDir, content: &str) {
        let path = settings_local_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn load_settings_enabled_defaults_to_true() {
        let dir = tempfile::tempdir().unwrap();

        // No settings file — defaults to enabled
        let settings = load_settings(dir.path()).unwrap();
        assert!(
            settings.enabled,
            "enabled should default to true when no settings file"
        );

        // Settings without enabled field — defaults to true
        fs::create_dir_all(dir.path().join(SETTINGS_DIR)).unwrap();
        fs::write(
            settings_path(dir.path()),
            r#"{"strategy": "manual-commit"}"#,
        )
        .unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert!(
            settings.enabled,
            "enabled should default to true when field is missing"
        );

        // Settings with enabled: false — should be false
        fs::write(
            settings_path(dir.path()),
            r#"{"strategy": "manual-commit", "enabled": false}"#,
        )
        .unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert!(
            !settings.enabled,
            "enabled should be false when explicitly set to false"
        );

        // Settings with enabled: true — should be true
        fs::write(
            settings_path(dir.path()),
            r#"{"strategy": "manual-commit", "enabled": true}"#,
        )
        .unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert!(
            settings.enabled,
            "enabled should be true when explicitly set to true"
        );
    }

    #[test]
    fn save_settings_preserves_enabled() {
        let dir = tempfile::tempdir().unwrap();

        let settings = BitloopsSettings {
            strategy: "manual-commit".into(),
            enabled: false,
            ..Default::default()
        };
        save_settings(&settings, &settings_path(dir.path())).unwrap();

        let loaded = load_settings(dir.path()).unwrap();
        assert!(
            !loaded.enabled,
            "enabled should be false after saving as false"
        );
    }

    #[test]
    fn is_enabled_test() {
        let dir = tempfile::tempdir().unwrap();

        // No settings file — defaults to true
        assert!(
            is_enabled(dir.path()).unwrap(),
            "should return true when no settings file"
        );

        fs::create_dir_all(dir.path().join(SETTINGS_DIR)).unwrap();

        // enabled: false
        fs::write(settings_path(dir.path()), r#"{"enabled": false}"#).unwrap();
        assert!(
            !is_enabled(dir.path()).unwrap(),
            "should return false when disabled"
        );

        // enabled: true
        fs::write(settings_path(dir.path()), r#"{"enabled": true}"#).unwrap();
        assert!(
            is_enabled(dir.path()).unwrap(),
            "should return true when enabled"
        );
    }

    #[test]
    fn load_settings_local_overrides_strategy() {
        let dir = setup();
        write_settings(&dir, r#"{"strategy": "manual-commit", "enabled": true}"#);
        write_local_settings(&dir, r#"{"strategy": "auto-commit"}"#);

        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(
            settings.strategy, "auto-commit",
            "local should override strategy"
        );
        assert!(settings.enabled, "base enabled should be preserved");
    }

    #[test]
    fn load_settings_local_overrides_enabled() {
        let dir = setup();
        write_settings(&dir, r#"{"strategy": "manual-commit", "enabled": true}"#);
        write_local_settings(&dir, r#"{"enabled": false}"#);

        let settings = load_settings(dir.path()).unwrap();
        assert!(!settings.enabled, "local should override enabled to false");
        assert_eq!(
            settings.strategy, "manual-commit",
            "base strategy should be preserved"
        );
    }

    #[test]
    fn load_settings_local_overrides_local_dev() {
        let dir = setup();
        write_settings(&dir, r#"{"strategy": "manual-commit"}"#);
        write_local_settings(&dir, r#"{"local_dev": true}"#);

        let settings = load_settings(dir.path()).unwrap();
        assert!(
            settings.local_dev,
            "local_dev should be true from local override"
        );
    }

    #[test]
    fn load_settings_local_merges_strategy_options() {
        let dir = setup();
        write_settings(
            &dir,
            r#"{"strategy": "manual-commit", "strategy_options": {"key1": "value1", "key2": "value2"}}"#,
        );
        write_local_settings(
            &dir,
            r#"{"strategy_options": {"key2": "overridden", "key3": "value3"}}"#,
        );

        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(
            settings.strategy_options["key1"].as_str(),
            Some("value1"),
            "key1 should remain from base"
        );
        assert_eq!(
            settings.strategy_options["key2"].as_str(),
            Some("overridden"),
            "key2 should be overridden by local"
        );
        assert_eq!(
            settings.strategy_options["key3"].as_str(),
            Some("value3"),
            "key3 should be added from local"
        );
    }

    #[test]
    fn load_settings_only_local_file_exists() {
        let dir = setup();
        // No base file, only local
        write_local_settings(&dir, r#"{"strategy": "auto-commit"}"#);

        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings.strategy, "auto-commit");
        assert!(settings.enabled, "enabled should default to true");
    }

    #[test]
    fn load_settings_no_local_file_uses_base() {
        let dir = setup();
        write_settings(&dir, r#"{"strategy": "manual-commit", "enabled": true}"#);

        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings.strategy, "manual-commit");
    }

    #[test]
    fn load_settings_empty_strategy_in_local_does_not_override() {
        let dir = setup();
        write_settings(&dir, r#"{"strategy": "manual-commit"}"#);
        write_local_settings(&dir, r#"{"strategy": ""}"#);

        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(
            settings.strategy, "manual-commit",
            "empty strategy in local should not override base"
        );
    }

    #[test]
    fn load_settings_neither_file_exists_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();

        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings.strategy, DEFAULT_STRATEGY);
        assert!(settings.enabled);
    }

    #[test]
    fn load_settings_rejects_unknown_keys_in_base() {
        let dir = setup();
        write_settings(&dir, r#"{"strategy": "manual-commit", "bogus_key": true}"#);

        let err = load_settings(dir.path()).unwrap_err();
        // Check full error chain (anyhow wraps the serde error with context)
        assert!(
            format!("{err:#}").contains("unknown field"),
            "expected 'unknown field' error, got: {err:#}"
        );
    }

    #[test]
    fn load_settings_rejects_unknown_keys_in_local() {
        let dir = setup();
        write_settings(&dir, r#"{"strategy": "manual-commit"}"#);
        write_local_settings(&dir, r#"{"bogus_key": "value"}"#);

        let err = load_settings(dir.path()).unwrap_err();
        // Check full error chain (anyhow wraps the serde error with context)
        assert!(
            format!("{err:#}").contains("unknown field"),
            "expected 'unknown field' error, got: {err:#}"
        );
    }

    #[test]
    fn load_rejects_unknown_keys() {
        let dir = setup();
        write_settings(
            &dir,
            r#"{"strategy": "manual-commit", "unknown_key": "value"}"#,
        );

        let err = load_settings(dir.path()).unwrap_err();
        assert!(
            format!("{err:#}").contains("unknown field"),
            "expected unknown-field error, got: {err:#}"
        );
    }

    #[test]
    fn load_accepts_valid_keys() {
        let dir = setup();
        write_settings(
            &dir,
            r#"{
                "strategy": "auto-commit",
                "enabled": true,
                "local_dev": false,
                "log_level": "debug",
                "strategy_options": {"key": "value"},
                "telemetry": true
            }"#,
        );

        let settings = load_settings(dir.path()).expect("expected valid settings to load");
        assert_eq!(settings.strategy, "auto-commit");
        assert!(settings.enabled);
        assert_eq!(settings.log_level, "debug");
        assert_eq!(settings.telemetry, Some(true));
    }

    #[test]
    fn load_local_settings_rejects_unknown_keys() {
        let dir = setup();
        write_settings(&dir, r#"{"strategy": "manual-commit"}"#);
        write_local_settings(&dir, r#"{"bad_key": true}"#);

        let err = load_settings(dir.path()).unwrap_err();
        assert!(
            format!("{err:#}").contains("unknown field"),
            "expected unknown-field error, got: {err:#}"
        );
    }

    // ── CLI-1469: unified config file pair ──────────────────────────────

    #[test]
    fn unified_config_file_constant_is_config_json() {
        assert_eq!(
            SETTINGS_FILE, "config.json",
            "SETTINGS_FILE should be 'config.json' for unified config model"
        );
    }

    #[test]
    fn unified_config_local_file_constant_is_config_local_json() {
        assert_eq!(
            SETTINGS_LOCAL_FILE, "config.local.json",
            "SETTINGS_LOCAL_FILE should be 'config.local.json' for unified config model"
        );
    }

    #[test]
    fn unified_config_settings_path_returns_config_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path());
        assert!(
            path.ends_with(".bitloops/config.json"),
            "settings_path should return .bitloops/config.json, got: {}",
            path.display()
        );
    }

    #[test]
    fn unified_config_settings_local_path_returns_config_local_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_local_path(dir.path());
        assert!(
            path.ends_with(".bitloops/config.local.json"),
            "settings_local_path should return .bitloops/config.local.json, got: {}",
            path.display()
        );
    }
}
