use super::*;
use crate::test_support::process_state::{enter_process_state, with_cwd};
use std::fs;

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
    let config_dir = temp.path().join(".bitloops");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.json"),
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
        })
        .to_string(),
    )
    .expect("write repo config");

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
fn semantic_config_reads_values_from_semantic_block() {
    let value = serde_json::json!({
        "semantic": {
            "provider": "openai",
            "model": "gpt-4.1-mini",
            "api_key": "file-key",
            "base_url": "http://localhost:11434/v1/chat/completions"
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);

    let cfg = resolve_store_semantic_config_for_tests(file_cfg, &[]);
    assert_eq!(cfg.semantic_provider.as_deref(), Some("openai"));
    assert_eq!(cfg.semantic_model.as_deref(), Some("gpt-4.1-mini"));
    assert_eq!(cfg.semantic_api_key.as_deref(), Some("file-key"));
    assert_eq!(
        cfg.semantic_base_url.as_deref(),
        Some("http://localhost:11434/v1/chat/completions")
    );
}

#[test]
fn knowledge_config_providers_defaults_when_block_missing() {
    let value = serde_json::json!({
        "stores": {
            "relational": { "provider": "sqlite" }
        }
    });

    let cfg = resolve_provider_config_for_tests(&value, &[]).expect("provider config");
    assert_eq!(cfg, ProviderConfig::default());
}

#[test]
fn knowledge_config_providers_reads_literal_values() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "github": { "token": "gh-token" },
                "atlassian": {
                    "site_url": "https://shared.atlassian.net",
                    "email": "shared@example.com",
                    "token": "shared-token"
                },
                "jira": {
                    "site_url": "https://bitloops.atlassian.net",
                    "email": "jira@example.com",
                    "token": "jira-token"
                },
                "confluence": {
                    "site_url": "https://bitloops.atlassian.net",
                    "email": "docs@example.com",
                    "token": "confluence-token"
                }
            }
        }
    });

    let cfg = resolve_provider_config_for_tests(&value, &[]).expect("provider config");
    assert_eq!(
        cfg.github,
        Some(GithubProviderConfig {
            token: "gh-token".to_string()
        })
    );
    assert_eq!(
        cfg.atlassian,
        Some(AtlassianProviderConfig {
            site_url: "https://shared.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        })
    );
    assert_eq!(
        cfg.jira,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "jira@example.com".to_string(),
            token: "jira-token".to_string(),
        })
    );
    assert_eq!(
        cfg.confluence,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "docs@example.com".to_string(),
            token: "confluence-token".to_string(),
        })
    );
}

#[test]
fn knowledge_config_providers_reads_shared_atlassian_values() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "atlassian": {
                    "site_url": "https://bitloops.atlassian.net",
                    "email": "shared@example.com",
                    "token": "shared-token"
                }
            }
        }
    });

    let cfg = resolve_provider_config_for_tests(&value, &[]).expect("provider config");
    assert_eq!(
        cfg.atlassian,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        })
    );
    assert_eq!(cfg.jira, None);
    assert_eq!(cfg.confluence, None);
}

#[test]
fn knowledge_config_providers_resolves_env_indirection() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "github": { "token": "${BITLOOPS_GITHUB_TOKEN}" }
            }
        }
    });

    let cfg = resolve_provider_config_for_tests(&value, &[("BITLOOPS_GITHUB_TOKEN", "env-gh")])
        .expect("provider config");
    assert_eq!(
        cfg.github,
        Some(GithubProviderConfig {
            token: "env-gh".to_string()
        })
    );
}

#[test]
fn knowledge_config_providers_shared_atlassian_resolves_env_indirection() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "atlassian": {
                    "site_url": "${BITLOOPS_ATLASSIAN_URL}",
                    "email": "${BITLOOPS_ATLASSIAN_EMAIL}",
                    "token": "${BITLOOPS_ATLASSIAN_TOKEN}"
                }
            }
        }
    });

    let cfg = resolve_provider_config_for_tests(
        &value,
        &[
            ("BITLOOPS_ATLASSIAN_URL", "https://bitloops.atlassian.net"),
            ("BITLOOPS_ATLASSIAN_EMAIL", "shared@example.com"),
            ("BITLOOPS_ATLASSIAN_TOKEN", "shared-token"),
        ],
    )
    .expect("provider config");
    assert_eq!(
        cfg.atlassian,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        })
    );
}

#[test]
fn knowledge_config_providers_rejects_missing_env_value() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "github": { "token": "${BITLOOPS_GITHUB_TOKEN}" }
            }
        }
    });

    let err = resolve_provider_config_for_tests(&value, &[]).expect_err("missing env should fail");
    assert!(err.to_string().contains("knowledge.providers.github.token"));
}

#[test]
fn knowledge_config_providers_rejects_missing_required_shared_atlassian_field() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "atlassian": {
                    "site_url": "https://bitloops.atlassian.net",
                    "email": "shared@example.com"
                }
            }
        }
    });

    let err = resolve_provider_config_for_tests(&value, &[])
        .expect_err("missing provider field should fail");
    assert!(
        err.to_string()
            .contains("missing `knowledge.providers.atlassian.token`")
    );
}

#[test]
fn knowledge_config_providers_rejects_missing_required_field() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "jira": {
                    "site_url": "https://bitloops.atlassian.net",
                    "email": "jira@example.com"
                }
            }
        }
    });

    let err = resolve_provider_config_for_tests(&value, &[])
        .expect_err("missing provider field should fail");
    assert!(
        err.to_string()
            .contains("missing `knowledge.providers.jira.token`")
    );
}

#[test]
fn knowledge_config_providers_jira_and_confluence_fall_back_to_shared_atlassian() {
    let cfg = ProviderConfig {
        github: None,
        atlassian: Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        }),
        jira: None,
        confluence: None,
    };

    assert_eq!(cfg.jira_config(), cfg.atlassian.as_ref());
    assert_eq!(cfg.confluence_config(), cfg.atlassian.as_ref());
}

#[test]
fn knowledge_config_providers_product_overrides_win_over_shared_atlassian() {
    let cfg = ProviderConfig {
        github: None,
        atlassian: Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        }),
        jira: Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "jira@example.com".to_string(),
            token: "jira-token".to_string(),
        }),
        confluence: Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "docs@example.com".to_string(),
            token: "docs-token".to_string(),
        }),
    };

    assert_eq!(cfg.jira_config(), cfg.jira.as_ref());
    assert_eq!(cfg.confluence_config(), cfg.confluence.as_ref());
}

#[test]
fn knowledge_config_providers_defaults_when_repo_config_missing() {
    let temp = tempfile::tempdir().expect("temp dir");

    let cfg = resolve_provider_config_for_repo(temp.path()).expect("provider config");

    assert_eq!(cfg, ProviderConfig::default());
}

#[test]
fn knowledge_config_providers_reads_values_from_repo_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_dir = temp.path().join(".bitloops");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.json"),
        serde_json::json!({
            "knowledge": {
                "providers": {
                    "github": { "token": "gh-token" },
                    "jira": {
                        "site_url": "https://bitloops.atlassian.net",
                        "email": "jira@example.com",
                        "token": "jira-token"
                    }
                }
            }
        })
        .to_string(),
    )
    .expect("write repo config");

    let cfg = resolve_provider_config_for_repo(temp.path()).expect("provider config");

    assert_eq!(
        cfg.github,
        Some(GithubProviderConfig {
            token: "gh-token".to_string(),
        })
    );
    assert_eq!(cfg.atlassian, None);
    assert_eq!(
        cfg.jira,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "jira@example.com".to_string(),
            token: "jira-token".to_string(),
        })
    );
    assert_eq!(cfg.confluence, None);
}

#[test]
fn knowledge_config_providers_resolve_from_current_repo_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_dir = temp.path().join(".bitloops");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.json"),
        serde_json::json!({
            "knowledge": {
                "providers": {
                    "confluence": {
                        "site_url": "https://bitloops.atlassian.net",
                        "email": "docs@example.com",
                        "token": "docs-token"
                    }
                }
            }
        })
        .to_string(),
    )
    .expect("write repo config");

    let _guard = enter_process_state(Some(temp.path()), &[]);
    let cfg = resolve_provider_config().expect("provider config");

    assert_eq!(
        cfg.confluence,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "docs@example.com".to_string(),
            token: "docs-token".to_string(),
        })
    );
    assert_eq!(cfg.github, None);
    assert_eq!(cfg.atlassian, None);
    assert_eq!(cfg.jira, None);
}

#[test]
fn knowledge_config_providers_reads_shared_atlassian_values_from_repo_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_dir = temp.path().join(".bitloops");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.json"),
        serde_json::json!({
            "knowledge": {
                "providers": {
                    "atlassian": {
                        "site_url": "https://bitloops.atlassian.net",
                        "email": "shared@example.com",
                        "token": "shared-token"
                    }
                }
            }
        })
        .to_string(),
    )
    .expect("write repo config");

    let cfg = resolve_provider_config_for_repo(temp.path()).expect("provider config");

    assert_eq!(
        cfg.atlassian,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        })
    );
    assert_eq!(cfg.github, None);
    assert_eq!(cfg.jira, None);
    assert_eq!(cfg.confluence, None);
}

#[test]
fn knowledge_config_providers_resolve_env_indirection_from_repo_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_dir = temp.path().join(".bitloops");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.json"),
        serde_json::json!({
            "knowledge": {
                "providers": {
                    "github": { "token": "${BITLOOPS_GITHUB_TOKEN}" }
                }
            }
        })
        .to_string(),
    )
    .expect("write repo config");

    let _guard = enter_process_state(None, &[("BITLOOPS_GITHUB_TOKEN", Some("env-gh-from-file"))]);
    let cfg = resolve_provider_config_for_repo(temp.path()).expect("provider config");

    assert_eq!(
        cfg.github,
        Some(GithubProviderConfig {
            token: "env-gh-from-file".to_string(),
        })
    );
}

#[test]
fn semantic_config_honors_env_over_file_precedence() {
    let value = serde_json::json!({
        "semantic": {
            "provider": "openai",
            "model": "gpt-4.1-mini",
            "api_key": "file-key"
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);
    let env = [
        (ENV_SEMANTIC_PROVIDER, "openai_compatible"),
        (ENV_SEMANTIC_MODEL, "qwen2.5-coder"),
        (ENV_SEMANTIC_API_KEY, "env-key"),
        (
            ENV_SEMANTIC_BASE_URL,
            "http://localhost:11434/v1/chat/completions",
        ),
    ];

    let cfg = resolve_store_semantic_config_for_tests(file_cfg, &env);
    assert_eq!(cfg.semantic_provider.as_deref(), Some("openai_compatible"));
    assert_eq!(cfg.semantic_model.as_deref(), Some("qwen2.5-coder"));
    assert_eq!(cfg.semantic_api_key.as_deref(), Some("env-key"));
    assert_eq!(
        cfg.semantic_base_url.as_deref(),
        Some("http://localhost:11434/v1/chat/completions")
    );
}

#[test]
fn semantic_config_resolves_from_current_repo_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_dir = temp.path().join(".bitloops");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.json"),
        serde_json::json!({
            "stores": {
                "semantic": {
                    "provider": "openai",
                    "model": "gpt-4.1-mini",
                    "api_key": "file-key",
                    "base_url": "http://localhost:11434/v1/chat/completions"
                }
            }
        })
        .to_string(),
    )
    .expect("write repo config");

    let _guard = enter_process_state(Some(temp.path()), &[]);
    let cfg = resolve_store_semantic_config();

    assert_eq!(cfg.semantic_provider.as_deref(), Some("openai"));
    assert_eq!(cfg.semantic_model.as_deref(), Some("gpt-4.1-mini"));
    assert_eq!(cfg.semantic_api_key.as_deref(), Some("file-key"));
    assert_eq!(
        cfg.semantic_base_url.as_deref(),
        Some("http://localhost:11434/v1/chat/completions")
    );
}

#[test]
fn events_backend_duckdb_path_defaults_under_repo_store_directory() {
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
        rendered.ends_with(".bitloops/stores/event/events.duckdb")
            || rendered.ends_with(".bitloops\\stores\\event\\events.duckdb")
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
fn events_backend_clickhouse_endpoint_uses_defaults() {
    let events = EventsBackendConfig {
        provider: EventsProvider::ClickHouse,
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
        provider: EventsProvider::ClickHouse,
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

#[test]
fn sqlite_path_resolution_uses_explicit_path() {
    let resolved = resolve_sqlite_db_path(Some("/tmp/bitloops-relational.sqlite"))
        .expect("explicit sqlite path should resolve");
    assert_eq!(resolved, PathBuf::from("/tmp/bitloops-relational.sqlite"));
}

#[test]
fn sqlite_path_resolution_resolves_relative_path_against_repo_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    with_cwd(temp.path(), || {
        let resolved = resolve_sqlite_db_path(Some("data/relational.sqlite"))
            .expect("relative sqlite path should resolve");
        assert!(
            resolved.ends_with(Path::new("data").join("relational.sqlite")),
            "expected repo-relative sqlite path, got {}",
            resolved.display()
        );
    });
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

    let expanded = expand_home_prefix_with(
        r"~\.bitloops\stores\relational\relational.db",
        Some(windows_home),
    )
    .expect("windows-style tilde sqlite path should resolve");

    assert_eq!(
        PathBuf::from(expanded),
        windows_home.join(r".bitloops\stores\relational\relational.db")
    );
}

#[test]
fn blob_local_path_resolution_defaults_under_current_repo_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _guard = enter_process_state(Some(temp.path()), &[]);

    let resolved = resolve_blob_local_path(None).expect("default blob path");
    let rendered = resolved.to_string_lossy();

    assert!(
        rendered.ends_with(".bitloops/stores/blob")
            || rendered.ends_with(".bitloops\\stores\\blob")
    );
}

#[test]
fn blob_storage_local_path_or_default_uses_current_repo_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = BlobStorageConfig {
        provider: BlobStorageProvider::Local,
        local_path: None,
        s3_bucket: None,
        s3_region: None,
        s3_access_key_id: None,
        s3_secret_access_key: None,
        gcs_bucket: None,
        gcs_credentials_path: None,
    };
    let _guard = enter_process_state(Some(temp.path()), &[]);

    let resolved = config
        .local_path_or_default()
        .expect("default local blob path");

    let rendered = resolved.to_string_lossy();
    assert!(
        rendered.ends_with(".bitloops/stores/blob")
            || rendered.ends_with(".bitloops\\stores\\blob")
    );
}

#[test]
fn dashboard_use_bitloops_local_reads_repo_config_via_public_helper() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_dir = temp.path().join(".bitloops");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.json"),
        serde_json::json!({
            "dashboard": {
                "use_bitloops_local": true
            }
        })
        .to_string(),
    )
    .expect("write repo config");

    let _guard = enter_process_state(Some(temp.path()), &[]);

    assert!(dashboard_use_bitloops_local());
    assert_eq!(DashboardFileConfig::load().use_bitloops_local, Some(true));
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
fn resolve_provider_config_defaults_without_repo_config_via_public_function() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _guard = enter_process_state(Some(temp.path()), &[]);

    let cfg = resolve_provider_config().expect("provider config");

    assert_eq!(cfg, ProviderConfig::default());
}

#[test]
fn dashboard_file_config_load_defaults_when_repo_config_missing() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _guard = enter_process_state(Some(temp.path()), &[]);

    assert_eq!(DashboardFileConfig::load(), DashboardFileConfig::default());
    assert!(!dashboard_use_bitloops_local());
}

#[test]
fn blob_local_path_resolution_uses_explicit_path() {
    let resolved =
        resolve_blob_local_path(Some("/tmp/bitloops-blobs")).expect("explicit blob path");
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
fn blob_local_path_resolution_defaults_under_repo_store_directory() {
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
    assert!(
        rendered.ends_with(".bitloops/stores/blob")
            || rendered.ends_with(".bitloops\\stores\\blob")
    );
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
        "stores": {
            "relational": {
                "provider": "sqlite"
            }
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
