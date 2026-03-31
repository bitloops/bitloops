use super::*;

#[test]
fn embedding_config_ignores_legacy_values_from_devql_file() {
    let value = serde_json::json!({
        "stores": {
            "embedding_provider": "voyage",
            "embedding_model": "voyage-code-3",
            "embedding_api_key": "voyage-key"
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);

    let cfg = resolve_store_embedding_config_for_tests(file_cfg, &[]);
    assert_eq!(cfg.embedding_provider, None);
    assert_eq!(cfg.embedding_model, None);
    assert_eq!(cfg.embedding_api_key, None);
}

#[test]
fn embedding_config_ignores_legacy_env_values() {
    let value = serde_json::json!({
        "stores": {
            "embedding_provider": "voyage",
            "embedding_model": "voyage-code-3",
            "embedding_api_key": "file-key"
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);

    let cfg = resolve_store_embedding_config_for_tests(file_cfg, &[]);
    assert_eq!(cfg.embedding_provider, None);
    assert_eq!(cfg.embedding_model, None);
    assert_eq!(cfg.embedding_api_key, None);
}

#[test]
fn embedding_config_does_not_default_provider_when_settings_exist() {
    let value = serde_json::json!({
        "stores": {
            "embedding_model": "jinaai/jina-embeddings-v2-base-code"
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);

    let cfg = resolve_store_embedding_config_for_tests(file_cfg, &[]);
    assert_eq!(cfg.embedding_provider, None);
    assert_eq!(cfg.embedding_model, None);
}

#[test]
fn embedding_config_defaults_to_disabled_when_no_embedding_settings_exist() {
    let cfg = resolve_store_embedding_config_for_tests(StoreFileConfig::default(), &[]);
    assert_eq!(cfg.embedding_provider, None);
    assert_eq!(cfg.embedding_model, None);
}

#[test]
fn resolve_store_embedding_config_ignores_file_and_env() {
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
        &[],
        || {
            let cfg = resolve_store_embedding_config();
            assert_eq!(cfg.embedding_provider, None);
            assert_eq!(cfg.embedding_model, None);
            assert_eq!(cfg.embedding_api_key, None);
        },
    );
}
