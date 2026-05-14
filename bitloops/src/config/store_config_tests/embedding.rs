use super::*;

#[test]
fn embedding_config_reads_values_from_devql_file() {
    let value = serde_json::json!({
        "stores": {
            "embedding_provider": "voyage",
            "embedding_model": "voyage-code-3",
            "embedding_api_key": "voyage-key"
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);

    let cfg = resolve_store_embedding_config_for_tests(file_cfg, &[]);
    assert_eq!(cfg.embedding_provider.as_deref(), Some("voyage"));
    assert_eq!(cfg.embedding_model.as_deref(), Some("voyage-code-3"));
    assert_eq!(cfg.embedding_api_key.as_deref(), Some("voyage-key"));
}

#[test]
fn embedding_config_honors_env_over_file_precedence() {
    let value = serde_json::json!({
        "stores": {
            "embedding_provider": "voyage",
            "embedding_model": "voyage-code-3",
            "embedding_api_key": "file-key"
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);
    let env = [
        (ENV_EMBEDDING_PROVIDER, "openai"),
        (ENV_EMBEDDING_MODEL, "text-embedding-3-large"),
        (ENV_EMBEDDING_API_KEY, "env-key"),
    ];

    let cfg = resolve_store_embedding_config_for_tests(file_cfg, &env);
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
        "stores": {
            "embedding_model": "jinaai/jina-embeddings-v2-base-code"
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);

    let cfg = resolve_store_embedding_config_for_tests(file_cfg, &[]);
    assert_eq!(cfg.embedding_provider.as_deref(), Some("local"));
    assert_eq!(
        cfg.embedding_model.as_deref(),
        Some("jinaai/jina-embeddings-v2-base-code")
    );
}

#[test]
fn embedding_config_defaults_provider_to_local_when_no_embedding_settings_exist() {
    let cfg = resolve_store_embedding_config_for_tests(StoreFileConfig::default(), &[]);
    assert_eq!(cfg.embedding_provider.as_deref(), Some("local"));
    assert_eq!(cfg.embedding_model, None);
}

#[test]
fn resolve_store_embedding_config_reads_file_and_env() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "stores": {
                "embedding_provider": "voyage",
                "embedding_model": "voyage-code-3",
                "embedding_api_key": "file-key"
            }
        }),
    );

    with_process_state(
        Some(temp.path()),
        &[
            (ENV_EMBEDDING_PROVIDER, Some("openai")),
            (ENV_EMBEDDING_MODEL, Some("text-embedding-3-large")),
            (ENV_EMBEDDING_API_KEY, Some("env-key")),
        ],
        || {
            let cfg = resolve_store_embedding_config();
            assert_eq!(cfg.embedding_provider.as_deref(), Some("openai"));
            assert_eq!(
                cfg.embedding_model.as_deref(),
                Some("text-embedding-3-large")
            );
        },
    );
}
