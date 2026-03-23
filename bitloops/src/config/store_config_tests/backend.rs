use super::*;

#[test]
fn backend_config_defaults_to_sqlite_duckdb_and_local_blob() {
    let cfg = resolve_store_backend_config_for_tests(StoreFileConfig::default()).expect("cfg");

    assert_eq!(cfg.relational.provider, RelationalProvider::Sqlite);
    assert_eq!(cfg.events.provider, EventsProvider::DuckDb);
    assert_eq!(cfg.blobs.provider, BlobStorageProvider::Local);
    assert_eq!(cfg.blobs.local_path, None);
    assert_eq!(cfg.blobs.s3_bucket, None);
    assert_eq!(cfg.blobs.gcs_bucket, None);
}

#[test]
fn backend_config_reads_store_blocks_from_repo_config_shape() {
    let value = serde_json::json!({
        "stores": {
            "relational": {
                "provider": "postgres",
                "postgres_dsn": "postgres://u:p@localhost:5432/bitloops"
            },
            "event": {
                "provider": "clickhouse",
                "clickhouse_url": "http://localhost:8123",
                "clickhouse_database": "bitloops"
            },
            "blob": {
                "provider": "gcs",
                "gcs_bucket": "bucket-a",
                "gcs_credentials_path": "/tmp/gcs.json"
            }
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);

    let cfg = resolve_store_backend_config_for_tests(file_cfg).expect("cfg");
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
    assert_eq!(cfg.blobs.provider, BlobStorageProvider::Gcs);
    assert_eq!(cfg.blobs.gcs_bucket.as_deref(), Some("bucket-a"));
    assert_eq!(
        cfg.blobs.gcs_credentials_path.as_deref(),
        Some("/tmp/gcs.json")
    );
}

#[test]
fn backend_config_rejects_invalid_provider_values() {
    let value = serde_json::json!({
        "stores": {
            "relational": { "provider": "mysql" },
            "event": { "provider": "kafka" }
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);

    let err =
        resolve_store_backend_config_for_tests(file_cfg).expect_err("invalid providers must fail");

    let message = err.to_string();
    assert!(message.contains("unsupported"));
}

#[test]
fn backend_config_resolves_from_current_repo_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "stores": {
                "relational": {
                    "provider": "postgres",
                    "postgres_dsn": "postgres://u:p@localhost:5432/bitloops"
                },
                "events": {
                    "provider": "clickhouse",
                    "clickhouse_url": "http://localhost:8123",
                    "clickhouse_database": "bitloops"
                },
                "blobs": {
                    "provider": "local",
                    "local_path": "data/blobs"
                }
            }
        }),
    );

    let _guard = enter_process_state(Some(temp.path()), &[]);
    let cfg = resolve_store_backend_config().expect("backend config");

    assert_eq!(cfg.relational.provider, RelationalProvider::Postgres);
    assert_eq!(
        cfg.relational.postgres_dsn.as_deref(),
        Some("postgres://u:p@localhost:5432/bitloops")
    );
    assert_eq!(cfg.events.provider, EventsProvider::ClickHouse);
    assert_eq!(cfg.events.clickhouse_database.as_deref(), Some("bitloops"));
    assert_eq!(cfg.blobs.provider, BlobStorageProvider::Local);
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
                    "provider": "sqlite",
                    "sqlite_path": "data/relational.sqlite"
                },
                "events": {
                    "provider": "duckdb",
                    "duckdb_path": "data/events.duckdb"
                }
            }
        })
        .to_string(),
    )
    .expect("write repo config");

    let _guard = enter_process_state(Some(temp.path()), &[]);
    let cfg = StoreFileConfig::load();

    assert_eq!(cfg.relational_provider.as_deref(), Some("sqlite"));
    assert_eq!(cfg.sqlite_path.as_deref(), Some("data/relational.sqlite"));
    assert_eq!(cfg.events_provider.as_deref(), Some("duckdb"));
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
                    "provider": "postgres",
                    "postgres_dsn": "postgres://user:pass@localhost:5432/bitloops"
                },
                "events": {
                    "provider": "clickhouse",
                    "clickhouse_url": "http://localhost:8123",
                    "clickhouse_database": "bitloops"
                },
                "blob": {
                    "provider": "local",
                    "local_path": "tmp/blobs"
                }
            }
        }),
    );

    with_cwd(temp.path(), || {
        let cfg = resolve_store_backend_config().expect("store backend config");
        assert_eq!(cfg.relational.provider, RelationalProvider::Postgres);
        assert_eq!(
            cfg.relational.postgres_dsn.as_deref(),
            Some("postgres://user:pass@localhost:5432/bitloops")
        );
        assert_eq!(cfg.events.provider, EventsProvider::ClickHouse);
        assert_eq!(cfg.events.clickhouse_database.as_deref(), Some("bitloops"));
        assert_eq!(cfg.blobs.provider, BlobStorageProvider::Local);
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
                    "provider": "sqlite",
                    "sqlite_path": "data/devql.sqlite"
                },
                "event": {
                    "provider": "duckdb",
                    "duckdb_path": "data/events.duckdb"
                }
            }
        }),
    );

    let cfg = resolve_store_backend_config_for_repo(temp.path()).expect("store backend config");
    assert_eq!(cfg.relational.provider, RelationalProvider::Sqlite);
    assert_eq!(
        cfg.relational.sqlite_path.as_deref(),
        Some("data/devql.sqlite")
    );
    assert_eq!(cfg.events.provider, EventsProvider::DuckDb);
    assert_eq!(
        cfg.events.duckdb_path.as_deref(),
        Some("data/events.duckdb")
    );
}
