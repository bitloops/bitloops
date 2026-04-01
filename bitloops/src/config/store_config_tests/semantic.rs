use super::*;

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
fn semantic_config_resolves_env_indirection_from_file_values() {
    let value = serde_json::json!({
        "semantic": {
            "provider": "${BITLOOPS_SEMANTIC_PROVIDER_FROM_FILE}",
            "model": "${BITLOOPS_SEMANTIC_MODEL_FROM_FILE}",
            "api_key": "${OPENAI_API_KEY}",
            "base_url": "${BITLOOPS_SEMANTIC_BASE_URL_FROM_FILE}"
        }
    });
    let file_cfg = StoreFileConfig::from_json_value(&value);
    let env = [
        ("BITLOOPS_SEMANTIC_PROVIDER_FROM_FILE", "openai_compatible"),
        ("BITLOOPS_SEMANTIC_MODEL_FROM_FILE", "qwen2.5-coder"),
        ("OPENAI_API_KEY", "indirected-key"),
        (
            "BITLOOPS_SEMANTIC_BASE_URL_FROM_FILE",
            "http://localhost:11434/v1/chat/completions",
        ),
    ];

    let cfg = resolve_store_semantic_config_for_tests(file_cfg, &env);
    assert_eq!(cfg.semantic_provider.as_deref(), Some("openai_compatible"));
    assert_eq!(cfg.semantic_model.as_deref(), Some("qwen2.5-coder"));
    assert_eq!(cfg.semantic_api_key.as_deref(), Some("indirected-key"));
    assert_eq!(
        cfg.semantic_base_url.as_deref(),
        Some("http://localhost:11434/v1/chat/completions")
    );
}

#[test]
fn resolve_store_semantic_config_reads_file_and_env() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "semantic": {
                "provider": "openai",
                "model": "gpt-4.1-mini",
                "api_key": "file-key",
                "base_url": "http://localhost:11434/v1/chat/completions"
            }
        }),
    );

    with_process_state(
        Some(temp.path()),
        &[
            (ENV_SEMANTIC_PROVIDER, Some("openai_compatible")),
            (ENV_SEMANTIC_MODEL, Some("qwen2.5-coder")),
            (ENV_SEMANTIC_API_KEY, Some("env-key")),
            (
                ENV_SEMANTIC_BASE_URL,
                Some("http://localhost:9999/v1/chat/completions"),
            ),
        ],
        || {
            let cfg = resolve_store_semantic_config();
            assert_eq!(cfg.semantic_provider.as_deref(), Some("openai_compatible"));
            assert_eq!(cfg.semantic_model.as_deref(), Some("qwen2.5-coder"));
            assert_eq!(cfg.semantic_api_key.as_deref(), Some("env-key"));
        },
    );
}
