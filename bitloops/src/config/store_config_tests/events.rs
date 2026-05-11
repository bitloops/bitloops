use super::*;

#[test]
fn events_backend_duckdb_path_defaults_under_test_state_store_directory() {
    let events = EventsBackendConfig {
        duckdb_path: None,
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    };

    let resolved = events.duckdb_path_or_default();
    assert_default_test_store_path(
        &resolved,
        None,
        &["data", "stores", "event", "events.duckdb"],
    );
}

#[test]
fn events_backend_duckdb_path_preserves_explicit_path() {
    let events = EventsBackendConfig {
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
fn events_backend_clickhouse_endpoint_uses_defaults() {
    let events = EventsBackendConfig {
        duckdb_path: None,
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    };

    assert_eq!(
        events.clickhouse_endpoint(),
        "http://localhost:8123/?database=default"
    );
}

#[test]
fn events_backend_clickhouse_endpoint_trims_trailing_slash() {
    let events = EventsBackendConfig {
        duckdb_path: None,
        clickhouse_url: Some("http://localhost:8123/".to_string()),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: Some("bitloops".to_string()),
    };

    assert_eq!(
        events.clickhouse_endpoint(),
        "http://localhost:8123/?database=bitloops"
    );
}
