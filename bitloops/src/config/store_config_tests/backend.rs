use super::*;

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
    assert_eq!(cfg.blobs.local_path.as_deref(), Some("data/blobs"));
}

#[test]
fn store_file_config_load_reads_repo_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_dir = temp.path().join(".bitloops");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.json"),
        serde_json::json!({
            "stores": {
                "relational": {

                    "sqlite_path": "data/relational.sqlite"
                },
                "events": {

                    "duckdb_path": "data/events.duckdb"
                }
            }
        })
        .to_string(),
    )
    .expect("write repo config");

    let _guard = enter_process_state(Some(temp.path()), &[]);
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
        assert_eq!(cfg.blobs.local_path.as_deref(), Some("tmp/blobs"));
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

    let cfg = resolve_store_backend_config_for_repo(temp.path()).expect("store backend config");
    assert!(!cfg.relational.has_postgres());
    assert_eq!(
        cfg.relational.sqlite_path.as_deref(),
        Some("data/devql.sqlite")
    );
    assert!(!cfg.events.has_clickhouse());
    assert_eq!(
        cfg.events.duckdb_path.as_deref(),
        Some("data/events.duckdb")
    );
}
