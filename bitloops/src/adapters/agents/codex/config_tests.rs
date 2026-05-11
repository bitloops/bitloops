use std::fs;
use std::path::{Path, PathBuf};

use super::*;

fn init_repo(path: &Path) {
    let output = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(path)
        .output()
        .expect("git init");
    assert!(output.status.success(), "git init should succeed");
}

fn config_file(path: &Path) -> PathBuf {
    path.join(".codex").join("config.toml")
}

fn read_config(path: &Path) -> String {
    fs::read_to_string(config_file(path)).expect("read config.toml")
}

#[test]
fn ensure_codex_hooks_feature_enabled_creates_repo_local_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let path = ensure_codex_hooks_feature_enabled_at(dir.path()).expect("ensure config");
    assert_eq!(path, config_file(dir.path()));
    assert!(path.exists(), "config.toml should be created");
    let raw = read_config(dir.path());
    assert!(raw.contains("[features]"));
    assert!(raw.contains("hooks = true"));
    assert!(!raw.contains("codex_hooks = true"));
}

#[test]
fn ensure_codex_hooks_feature_enabled_preserves_existing_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let codex_dir = dir.path().join(".codex");
    fs::create_dir_all(&codex_dir).expect("create .codex");
    fs::write(
        config_file(dir.path()),
        r#"
# codex config
[profile]
name = "strict"
"#,
    )
    .expect("seed config");

    ensure_codex_hooks_feature_enabled_at(dir.path()).expect("ensure config");
    let raw = read_config(dir.path());
    assert!(raw.contains("strict"));
    assert!(raw.contains("hooks = true"));
    assert!(!raw.contains("codex_hooks = true"));
}

#[test]
fn ensure_codex_hooks_feature_enabled_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    ensure_codex_hooks_feature_enabled_at(dir.path()).expect("first ensure");
    let first = read_config(dir.path());

    ensure_codex_hooks_feature_enabled_at(dir.path()).expect("second ensure");
    let second = read_config(dir.path());

    assert_eq!(first, second);
}

#[test]
fn ensure_codex_hooks_feature_enabled_migrates_legacy_codex_hooks_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let codex_dir = dir.path().join(".codex");
    fs::create_dir_all(&codex_dir).expect("create .codex");
    fs::write(config_file(dir.path()), "[features]\ncodex_hooks = true\n").expect("seed config");

    ensure_codex_hooks_feature_enabled_at(dir.path()).expect("ensure config");
    let raw = read_config(dir.path());
    assert!(raw.contains("hooks = true"));
    assert!(!raw.contains("codex_hooks = true"));
}

#[test]
fn codex_hooks_feature_enabled_accepts_legacy_codex_hooks_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let codex_dir = dir.path().join(".codex");
    fs::create_dir_all(&codex_dir).expect("create .codex");
    fs::write(config_file(dir.path()), "[features]\ncodex_hooks = true\n").expect("seed config");

    assert!(codex_hooks_feature_enabled_at(dir.path()));
}

#[test]
fn codex_hooks_feature_enabled_prefers_canonical_hooks_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let codex_dir = dir.path().join(".codex");
    fs::create_dir_all(&codex_dir).expect("create .codex");
    fs::write(
        config_file(dir.path()),
        "[features]\nhooks = false\ncodex_hooks = true\n",
    )
    .expect("seed config");

    assert!(!codex_hooks_feature_enabled_at(dir.path()));
}

#[test]
fn ensure_codex_hooks_feature_enabled_refuses_to_clobber_non_table_features() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let codex_dir = dir.path().join(".codex");
    fs::create_dir_all(&codex_dir).expect("create .codex");
    let original = r#"
# codex config
features = true
[profile]
name = "strict"
"#;
    fs::write(config_file(dir.path()), original).expect("seed config");

    let err =
        ensure_codex_hooks_feature_enabled_at(dir.path()).expect_err("should refuse to clobber");
    assert!(
        err.to_string().contains("failed to parse Codex config")
            || err.to_string().contains("features")
    );
    assert_eq!(read_config(dir.path()), original);
}
