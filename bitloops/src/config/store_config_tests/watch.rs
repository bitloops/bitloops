use super::*;

#[test]
fn watch_file_config_reads_json_root_keys() {
    let value = serde_json::json!({
        "watch_debounce_ms": 750,
        "watch_poll_fallback_ms": 2500
    });

    let cfg = WatchFileConfig::from_json_value(&value);
    assert_eq!(cfg.watch_debounce_ms, Some(750));
    assert_eq!(cfg.watch_poll_fallback_ms, Some(2500));
}

#[test]
fn watch_file_config_reads_json_watch_block() {
    let value = serde_json::json!({
        "watch": {
            "watch_debounce_ms": "900",
            "watch_poll_fallback_ms": 2600
        }
    });

    let cfg = WatchFileConfig::from_json_value(&value);
    assert_eq!(cfg.watch_debounce_ms, Some(900));
    assert_eq!(cfg.watch_poll_fallback_ms, Some(2600));
}

#[test]
fn watch_runtime_config_prefers_env_over_file() {
    let cfg = resolve_watch_runtime_config_for_tests(
        WatchFileConfig {
            watch_debounce_ms: Some(500),
            watch_poll_fallback_ms: Some(2000),
        },
        &[
            (ENV_WATCH_DEBOUNCE_MS, "850"),
            (ENV_WATCH_POLL_FALLBACK_MS, "3100"),
        ],
    );

    assert_eq!(cfg.watch_debounce_ms, 850);
    assert_eq!(cfg.watch_poll_fallback_ms, 3100);
}
