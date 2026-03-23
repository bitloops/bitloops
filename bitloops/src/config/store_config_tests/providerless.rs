// ---------------------------------------------------------------------------
// Provider-less config model tests (spec §5.1, CLI-1480)
//
// These tests assert the target API where provider enums are removed and
// backend availability is derived from connection-string presence.
// They MUST fail against the current codebase (proving the gap).
// ---------------------------------------------------------------------------

use super::*;

#[test]
fn providerless_relational_has_postgres_true_when_dsn_present() {
    let cfg = RelationalBackendConfig {
        sqlite_path: Some(".bitloops/stores/relational/relational.db".to_string()),
        postgres_dsn: Some("postgres://u:p@localhost:5432/bitloops".to_string()),
    };
    assert!(
        cfg.has_postgres(),
        "has_postgres should be true when postgres_dsn is set"
    );
}

#[test]
fn providerless_relational_has_postgres_false_when_dsn_absent() {
    let cfg = RelationalBackendConfig {
        sqlite_path: Some(".bitloops/stores/relational/relational.db".to_string()),
        postgres_dsn: None,
    };
    assert!(
        !cfg.has_postgres(),
        "has_postgres should be false when postgres_dsn is absent"
    );
}

#[test]
fn providerless_events_has_clickhouse_true_when_url_present() {
    let cfg = EventsBackendConfig {
        duckdb_path: None,
        clickhouse_url: Some("http://localhost:8123".to_string()),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    };
    assert!(
        cfg.has_clickhouse(),
        "has_clickhouse should be true when clickhouse_url is set"
    );
}

#[test]
fn providerless_events_has_clickhouse_false_when_url_absent() {
    let cfg = EventsBackendConfig {
        duckdb_path: None,
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    };
    assert!(
        !cfg.has_clickhouse(),
        "has_clickhouse should be false when clickhouse_url is absent"
    );
}

#[test]
fn providerless_blob_has_remote_true_when_s3_configured() {
    let cfg = BlobStorageConfig {
        local_path: None,
        s3_bucket: Some("my-bucket".to_string()),
        s3_region: Some("us-east-1".to_string()),
        s3_access_key_id: None,
        s3_secret_access_key: None,
        gcs_bucket: None,
        gcs_credentials_path: None,
    };
    assert!(
        cfg.has_remote(),
        "has_remote should be true when s3_bucket is set"
    );
}

#[test]
fn providerless_blob_has_remote_true_when_gcs_configured() {
    let cfg = BlobStorageConfig {
        local_path: None,
        s3_bucket: None,
        s3_region: None,
        s3_access_key_id: None,
        s3_secret_access_key: None,
        gcs_bucket: Some("my-gcs-bucket".to_string()),
        gcs_credentials_path: Some("/path/to/creds.json".to_string()),
    };
    assert!(
        cfg.has_remote(),
        "has_remote should be true when gcs_bucket is set"
    );
}

#[test]
fn providerless_blob_has_remote_false_when_no_remote() {
    let cfg = BlobStorageConfig {
        local_path: None,
        s3_bucket: None,
        s3_region: None,
        s3_access_key_id: None,
        s3_secret_access_key: None,
        gcs_bucket: None,
        gcs_credentials_path: None,
    };
    assert!(
        !cfg.has_remote(),
        "has_remote should be false when no remote backend is configured"
    );
}

#[test]
fn providerless_resolve_produces_valid_config_from_json_without_provider() {
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
                "s3_bucket": "my-bucket",
                "s3_region": "us-east-1"
            }
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);
    let cfg = resolve_store_backend_config_for_tests(file_cfg)
        .expect("resolve should succeed without provider keys");

    assert!(
        cfg.relational.has_postgres(),
        "postgres_dsn present → has_postgres true"
    );
    assert!(
        cfg.events.has_clickhouse(),
        "clickhouse_url present → has_clickhouse true"
    );
    assert!(
        cfg.blobs.has_remote(),
        "s3_bucket present → has_remote true"
    );
}

#[test]
fn providerless_sqlite_path_always_has_default_value() {
    let cfg = resolve_store_backend_config_for_tests(StoreFileConfig::default()).expect("cfg");
    assert!(
        cfg.relational.sqlite_path.is_some(),
        "sqlite_path must always have a default value in the resolved config"
    );
}
