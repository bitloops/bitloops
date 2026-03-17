use super::*;

#[test]
fn backend_config_defaults_to_sqlite_and_duckdb() {
    let cfg = resolve_devql_backend_config_for_tests(DevqlFileConfig::default(), &[]).expect("cfg");

    assert_eq!(cfg.relational.provider, RelationalProvider::Sqlite);
    assert_eq!(cfg.events.provider, EventsProvider::DuckDb);
    assert_eq!(cfg.blobs.provider, BlobStorageProvider::Local);
    assert_eq!(cfg.blobs.local_path, None);
    assert_eq!(cfg.blobs.s3_bucket, None);
    assert_eq!(cfg.blobs.gcs_bucket, None);
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
            "blobs": {
                "provider": "gcs",
                "gcs_bucket": "file-gcs-bucket",
                "gcs_credentials_path": "/tmp/file-gcs-creds.json"
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
        (ENV_BLOB_STORAGE_PROVIDER, "s3"),
        (ENV_BLOB_S3_BUCKET, "env-s3-bucket"),
        (ENV_BLOB_S3_REGION, "eu-west-1"),
        (ENV_BLOB_S3_ACCESS_KEY_ID, "env-access-key"),
        (ENV_BLOB_S3_SECRET_ACCESS_KEY, "env-secret-key"),
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
    assert_eq!(cfg.blobs.provider, BlobStorageProvider::S3);
    assert_eq!(cfg.blobs.s3_bucket.as_deref(), Some("env-s3-bucket"));
    assert_eq!(cfg.blobs.s3_region.as_deref(), Some("eu-west-1"));
    assert_eq!(
        cfg.blobs.s3_access_key_id.as_deref(),
        Some("env-access-key")
    );
    assert_eq!(
        cfg.blobs.s3_secret_access_key.as_deref(),
        Some("env-secret-key")
    );
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
fn semantic_config_reads_values_from_devql_file() {
    let value = serde_json::json!({
        "devql": {
            "semantic_provider": "openai",
            "semantic_model": "gpt-4.1-mini",
            "semantic_api_key": "file-key",
            "semantic_base_url": "http://localhost:11434/v1/chat/completions"
        }
    });
    let file_cfg = DevqlFileConfig::from_json_value(&value);

    let cfg = resolve_devql_semantic_config_for_tests(file_cfg, &[]);
    assert_eq!(cfg.semantic_provider.as_deref(), Some("openai"));
    assert_eq!(cfg.semantic_model.as_deref(), Some("gpt-4.1-mini"));
    assert_eq!(cfg.semantic_api_key.as_deref(), Some("file-key"));
    assert_eq!(
        cfg.semantic_base_url.as_deref(),
        Some("http://localhost:11434/v1/chat/completions")
    );
}

#[test]
fn semantic_config_honors_env_over_file_precedence() {
    let value = serde_json::json!({
        "devql": {
            "semantic_provider": "openai",
            "semantic_model": "gpt-4.1-mini",
            "semantic_api_key": "file-key"
        }
    });
    let file_cfg = DevqlFileConfig::from_json_value(&value);
    let env = [
        (ENV_SEMANTIC_PROVIDER, "openai_compatible"),
        (ENV_SEMANTIC_MODEL, "qwen2.5-coder"),
        (ENV_SEMANTIC_API_KEY, "env-key"),
        (
            ENV_SEMANTIC_BASE_URL,
            "http://localhost:11434/v1/chat/completions",
        ),
    ];

    let cfg = resolve_devql_semantic_config_for_tests(file_cfg, &env);
    assert_eq!(cfg.semantic_provider.as_deref(), Some("openai_compatible"));
    assert_eq!(cfg.semantic_model.as_deref(), Some("qwen2.5-coder"));
    assert_eq!(cfg.semantic_api_key.as_deref(), Some("env-key"));
    assert_eq!(
        cfg.semantic_base_url.as_deref(),
        Some("http://localhost:11434/v1/chat/completions")
    );
}

#[test]
fn embedding_config_reads_values_from_devql_file() {
    let value = serde_json::json!({
        "devql": {
            "embedding_provider": "voyage",
            "embedding_model": "voyage-code-3",
            "embedding_api_key": "voyage-key"
        }
    });
    let file_cfg = DevqlFileConfig::from_json_value(&value);

    let cfg = resolve_devql_embedding_config_for_tests(file_cfg, &[]);
    assert_eq!(cfg.embedding_provider.as_deref(), Some("voyage"));
    assert_eq!(cfg.embedding_model.as_deref(), Some("voyage-code-3"));
    assert_eq!(cfg.embedding_api_key.as_deref(), Some("voyage-key"));
}

#[test]
fn embedding_config_honors_env_over_file_precedence() {
    let value = serde_json::json!({
        "devql": {
            "embedding_provider": "voyage",
            "embedding_model": "voyage-code-3",
            "embedding_api_key": "file-key"
        }
    });
    let file_cfg = DevqlFileConfig::from_json_value(&value);
    let env = [
        (ENV_EMBEDDING_PROVIDER, "openai"),
        (ENV_EMBEDDING_MODEL, "text-embedding-3-large"),
        (ENV_EMBEDDING_API_KEY, "env-key"),
    ];

    let cfg = resolve_devql_embedding_config_for_tests(file_cfg, &env);
    assert_eq!(cfg.embedding_provider.as_deref(), Some("openai"));
    assert_eq!(
        cfg.embedding_model.as_deref(),
        Some("text-embedding-3-large")
    );
    assert_eq!(cfg.embedding_api_key.as_deref(), Some("env-key"));
}

#[test]
fn embedding_config_defaults_provider_to_local_when_settings_exist() {
    let value = serde_json::json!({
        "devql": {
            "embedding_model": "jinaai/jina-embeddings-v2-base-code"
        }
    });
    let file_cfg = DevqlFileConfig::from_json_value(&value);

    let cfg = resolve_devql_embedding_config_for_tests(file_cfg, &[]);
    assert_eq!(cfg.embedding_provider.as_deref(), Some("local"));
    assert_eq!(
        cfg.embedding_model.as_deref(),
        Some("jinaai/jina-embeddings-v2-base-code")
    );
}

#[test]
fn embedding_config_defaults_provider_to_local_when_no_embedding_settings_exist() {
    let cfg = resolve_devql_embedding_config_for_tests(DevqlFileConfig::default(), &[]);
    assert_eq!(cfg.embedding_provider.as_deref(), Some("local"));
    assert_eq!(cfg.embedding_model, None);
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

    let resolved =
        resolve_sqlite_db_path(Some("~/devql.sqlite")).expect("tilde sqlite path should resolve");
    assert_eq!(resolved, home.join("devql.sqlite"));
}

#[test]
fn sqlite_path_resolution_expands_windows_tilde_prefix_with_windows_home() {
    let windows_home = Path::new(r"C:\Users\bitloops");

    let expanded = expand_home_prefix_with(r"~\.bitloops\devql\relational.db", Some(windows_home))
        .expect("windows-style tilde sqlite path should resolve");

    assert_eq!(
        PathBuf::from(expanded),
        windows_home.join(r".bitloops\devql\relational.db")
    );
}

#[test]
fn blob_local_path_resolution_uses_explicit_path() {
    let resolved = resolve_blob_local_path(Some("/tmp/bitloops-blobs"))
        .expect("explicit blob path should resolve");
    assert_eq!(resolved, PathBuf::from("/tmp/bitloops-blobs"));
}

#[test]
fn blob_local_path_resolution_expands_tilde_prefix() {
    let Some(home) = user_home_dir() else {
        return;
    };

    let resolved =
        resolve_blob_local_path(Some("~/blob-storage")).expect("tilde blob path should resolve");
    assert_eq!(resolved, home.join("blob-storage"));
}

#[test]
fn blob_local_path_resolution_defaults_under_bitloops_directory() {
    let blobs = BlobStorageConfig {
        provider: BlobStorageProvider::Local,
        local_path: None,
        s3_bucket: None,
        s3_region: None,
        s3_access_key_id: None,
        s3_secret_access_key: None,
        gcs_bucket: None,
        gcs_credentials_path: None,
    };

    let resolved = blobs
        .local_path_or_default()
        .expect("default local blob path");
    let rendered = resolved.to_string_lossy();
    assert!(rendered.ends_with(".bitloops/blobs") || rendered.ends_with(".bitloops\\blobs"));
}

#[test]
fn dashboard_file_config_reads_use_bitloops_local_flag() {
    let value = serde_json::json!({
        "dashboard": {
            "use_bitloops_local": true
        }
    });

    let cfg = DashboardFileConfig::from_json_value(&value);
    assert_eq!(cfg.use_bitloops_local, Some(true));
}

#[test]
fn dashboard_file_config_defaults_when_dashboard_block_missing() {
    let value = serde_json::json!({
        "devql": {
            "relational_provider": "sqlite"
        }
    });

    let cfg = DashboardFileConfig::from_json_value(&value);
    assert_eq!(cfg, DashboardFileConfig::default());
}

#[test]
fn dashboard_file_config_accepts_boolean_like_strings() {
    let value = serde_json::json!({
        "dashboard": {
            "use_bitloops_local": "yes"
        }
    });

    let cfg = DashboardFileConfig::from_json_value(&value);
    assert_eq!(cfg.use_bitloops_local, Some(true));
}
