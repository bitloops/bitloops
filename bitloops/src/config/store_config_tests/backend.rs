use super::*;
use crate::test_support::process_state::git_command;

fn resolved_path(root: &Path, relative: &str) -> String {
    let candidate = root.join(relative);
    let canonical_path = if candidate.extension().is_some() {
        let parent = candidate.parent().expect("file path parent");
        fs::create_dir_all(parent).expect("create parent directory");
        parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf())
            .join(candidate.file_name().expect("file name"))
    } else {
        fs::create_dir_all(&candidate).expect("create directory");
        candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.clone())
    };
    canonical_path.to_string_lossy().to_string()
}

#[test]
fn backend_config_defaults_to_sqlite_duckdb_and_local_blob() {
    let cfg = resolve_store_backend_config_for_tests(StoreFileConfig::default()).expect("cfg");

    assert!(!cfg.relational.has_postgres());
    assert!(!cfg.events.has_clickhouse());
    assert!(!cfg.blobs.has_remote());
    assert!(cfg.blobs.local_path.is_some(), "local_path should default");
    assert_eq!(cfg.blobs.s3_bucket, None);
    assert_eq!(cfg.blobs.gcs_bucket, None);
}

#[test]
fn backend_config_reads_store_blocks_from_repo_config_shape() {
    let value = serde_json::json!({
        "stores": {
            "relational": {

                "postgres_dsn": "postgres://u:p@localhost:5432/bitloops"
            },
            "event": {

                "clickhouse_url": "http://localhost:8123",
                "clickhouse_database": "bitloops"
            },
            "blob": {

                "gcs_bucket": "bucket-a",
                "gcs_credentials_path": "/tmp/gcs.json"
            }
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);

    let cfg = resolve_store_backend_config_for_tests(file_cfg).expect("cfg");
    assert!(cfg.relational.has_postgres());
    assert!(cfg.events.has_clickhouse());
    assert_eq!(
        cfg.relational.postgres_dsn.as_deref(),
        Some("postgres://u:p@localhost:5432/bitloops")
    );
    assert_eq!(
        cfg.events.clickhouse_url.as_deref(),
        Some("http://localhost:8123")
    );
    assert_eq!(cfg.events.clickhouse_database.as_deref(), Some("bitloops"));
    assert!(cfg.blobs.has_remote());
    assert_eq!(cfg.blobs.gcs_bucket.as_deref(), Some("bucket-a"));
    assert_eq!(
        cfg.blobs.gcs_credentials_path.as_deref(),
        Some("/tmp/gcs.json")
    );
}

#[test]
fn backend_config_resolves_from_current_repo_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "stores": {
                "relational": {

                    "postgres_dsn": "postgres://u:p@localhost:5432/bitloops"
                },
                "events": {

                    "clickhouse_url": "http://localhost:8123",
                    "clickhouse_database": "bitloops"
                },
                "blobs": {

                    "local_path": "data/blobs"
                }
            }
        }),
    );

    let _guard = enter_process_state(Some(temp.path()), &[]);
    let cfg = resolve_store_backend_config().expect("backend config");

    assert!(cfg.relational.has_postgres());
    assert_eq!(
        cfg.relational.postgres_dsn.as_deref(),
        Some("postgres://u:p@localhost:5432/bitloops")
    );
    assert!(cfg.events.has_clickhouse());
    assert_eq!(cfg.events.clickhouse_database.as_deref(), Some("bitloops"));
    assert!(!cfg.blobs.has_remote());
    assert_eq!(
        cfg.blobs.local_path.as_deref(),
        Some(resolved_path(temp.path(), "data/blobs").as_str())
    );
}

#[test]
fn store_file_config_load_reads_daemon_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_root = temp.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(config_root.as_str()),
        )],
    );
    write_repo_config(
        &temp.path().join("bitloops"),
        serde_json::json!({
            "stores": {
                "relational": {
                    "sqlite_path": "data/relational.sqlite"
                },
                "events": {
                    "duckdb_path": "data/events.duckdb"
                }
            }
        }),
    );
    let cfg = StoreFileConfig::load();

    assert_eq!(cfg.sqlite_path.as_deref(), Some("data/relational.sqlite"));
    assert_eq!(cfg.duckdb_path.as_deref(), Some("data/events.duckdb"));
}

#[test]
fn resolve_store_backend_config_reads_repo_config_from_current_dir() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "stores": {
                "relational": {

                    "postgres_dsn": "postgres://user:pass@localhost:5432/bitloops"
                },
                "events": {

                    "clickhouse_url": "http://localhost:8123",
                    "clickhouse_database": "bitloops"
                },
                "blob": {

                    "local_path": "tmp/blobs"
                }
            }
        }),
    );

    with_cwd(temp.path(), || {
        let cfg = resolve_store_backend_config().expect("store backend config");
        assert!(cfg.relational.has_postgres());
        assert_eq!(
            cfg.relational.postgres_dsn.as_deref(),
            Some("postgres://user:pass@localhost:5432/bitloops")
        );
        assert!(cfg.events.has_clickhouse());
        assert_eq!(cfg.events.clickhouse_database.as_deref(), Some("bitloops"));
        assert!(!cfg.blobs.has_remote());
        assert_eq!(
            cfg.blobs.local_path.as_deref(),
            Some(resolved_path(temp.path(), "tmp/blobs").as_str())
        );
    });
}

#[test]
fn resolve_store_backend_config_for_repo_uses_repo_root_parameter() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "stores": {
                "relational": {

                    "sqlite_path": "data/devql.sqlite"
                },
                "event": {

                    "duckdb_path": "data/events.duckdb"
                }
            }
        }),
    );
    let _guard = enter_process_state(None, &[]);

    let cfg = resolve_store_backend_config_for_repo(temp.path()).expect("store backend config");
    assert!(!cfg.relational.has_postgres());
    assert_eq!(
        cfg.relational.sqlite_path.as_deref(),
        Some(
            temp.path()
                .join("data/devql.sqlite")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert!(!cfg.events.has_clickhouse());
    assert_eq!(
        cfg.events.duckdb_path.as_deref(),
        Some(
            temp.path()
                .join("data/events.duckdb")
                .to_string_lossy()
                .as_ref()
        )
    );
}

#[test]
fn resolve_store_backend_config_honours_explicit_daemon_config_override_inside_git_repo() {
    let temp = tempfile::tempdir().expect("temp dir");
    let status = git_command()
        .args(["init"])
        .current_dir(temp.path())
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");

    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "stores": {
                "relational": {
                    "postgres_dsn": "postgres://repo-root"
                },
                "events": {
                    "clickhouse_url": "http://repo-root-clickhouse:8123",
                    "clickhouse_database": "repo_root"
                }
            }
        }),
    );

    let nested = temp.path().join("test-runs").join("case-a");
    fs::create_dir_all(&nested).expect("create nested config dir");
    write_envelope_config(
        &nested,
        serde_json::json!({
            "stores": {
                "relational": {
                    "sqlite_path": "stores/relational/relational.db"
                },
                "events": {
                    "duckdb_path": "stores/event/events.duckdb"
                }
            }
        }),
    );

    let config_path = nested.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let config_path_string = config_path.to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(nested.as_path()),
        &[(
            ENV_DAEMON_CONFIG_PATH_OVERRIDE,
            Some(config_path_string.as_str()),
        )],
    );

    let cfg = resolve_store_backend_config().expect("store backend config");
    assert!(!cfg.relational.has_postgres());
    assert_eq!(
        cfg.relational.sqlite_path.as_deref(),
        Some(
            nested
                .join("stores/relational/relational.db")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert!(!cfg.events.has_clickhouse());
    assert_eq!(
        cfg.events.duckdb_path.as_deref(),
        Some(
            nested
                .join("stores/event/events.duckdb")
                .to_string_lossy()
                .as_ref()
        )
    );
}
