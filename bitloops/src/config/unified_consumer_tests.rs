use serde_json::json;

use super::unified_config::{
    UnifiedSettings, merge_layers, resolve_dashboard_from_unified, resolve_embedding_from_unified,
    resolve_provider_from_unified, resolve_semantic_from_unified,
    resolve_store_backend_from_unified, resolve_watch_from_unified,
};
use super::{
    ENV_SEMANTIC_API_KEY, ENV_SEMANTIC_BASE_URL, ENV_SEMANTIC_MODEL, ENV_SEMANTIC_PROVIDER,
    ENV_WATCH_DEBOUNCE_MS, ENV_WATCH_POLL_FALLBACK_MS,
};

fn no_env(_key: &str) -> Option<String> {
    None
}

// ---------------------------------------------------------------------------
// A. Store backend from unified config
// ---------------------------------------------------------------------------

#[test]
fn store_backend_from_unified_reads_relational_and_events() {
    let settings = UnifiedSettings {
        stores: Some(json!({
            "relational": { "postgres_dsn": "postgres://localhost/db" },
            "events": { "duckdb_path": "data/events.duckdb" }
        })),
        ..Default::default()
    };
    let tmp = tempfile::tempdir().unwrap();
    let cfg = resolve_store_backend_from_unified(&settings, tmp.path()).unwrap();

    assert!(cfg.relational.has_postgres());
    assert_eq!(
        cfg.relational.postgres_dsn.as_deref(),
        Some("postgres://localhost/db")
    );
    assert!(!cfg.events.has_clickhouse());
}

#[test]
fn store_backend_from_unified_applies_defaults() {
    let settings = UnifiedSettings::default(); // no stores block
    let tmp = tempfile::tempdir().unwrap();
    let cfg = resolve_store_backend_from_unified(&settings, tmp.path()).unwrap();

    assert!(!cfg.relational.has_postgres());
    assert!(!cfg.events.has_clickhouse());
    assert!(!cfg.blobs.has_remote());
}

#[test]
fn store_backend_from_unified_merges_across_layers() {
    let global = UnifiedSettings {
        stores: Some(json!({
            "relational": { "sqlite_path": "data/relational.db" }
        })),
        ..Default::default()
    };
    let project = UnifiedSettings {
        stores: Some(json!({
            "events": { "clickhouse_url": "http://ch:8123" }
        })),
        ..Default::default()
    };
    let merged = merge_layers(&[global, project]);
    let tmp = tempfile::tempdir().unwrap();
    let cfg = resolve_store_backend_from_unified(&merged, tmp.path()).unwrap();

    assert!(!cfg.relational.has_postgres());
    assert!(cfg.events.has_clickhouse());
    assert_eq!(cfg.events.clickhouse_url.as_deref(), Some("http://ch:8123"));
}

// ---------------------------------------------------------------------------
// B. Semantic config from unified
// ---------------------------------------------------------------------------

#[test]
fn semantic_from_unified_reads_semantic_block() {
    let settings = UnifiedSettings {
        semantic: Some(json!({
            "provider": "openai",
            "model": "text-embedding-3-small",
            "api_key": "sk-test",
            "base_url": "https://api.openai.com"
        })),
        ..Default::default()
    };
    let cfg = resolve_semantic_from_unified(&settings, no_env);

    assert_eq!(cfg.semantic_provider.as_deref(), Some("openai"));
    assert_eq!(
        cfg.semantic_model.as_deref(),
        Some("text-embedding-3-small")
    );
    assert_eq!(cfg.semantic_api_key.as_deref(), Some("sk-test"));
    assert_eq!(
        cfg.semantic_base_url.as_deref(),
        Some("https://api.openai.com")
    );
}

#[test]
fn semantic_from_unified_env_wins_over_file() {
    let settings = UnifiedSettings {
        semantic: Some(json!({
            "provider": "openai",
            "model": "file-model"
        })),
        ..Default::default()
    };
    let cfg = resolve_semantic_from_unified(&settings, |key| match key {
        k if k == ENV_SEMANTIC_PROVIDER => Some("anthropic".into()),
        k if k == ENV_SEMANTIC_MODEL => Some("env-model".into()),
        k if k == ENV_SEMANTIC_API_KEY => Some("env-key".into()),
        k if k == ENV_SEMANTIC_BASE_URL => Some("https://env.example.com".into()),
        _ => None,
    });

    assert_eq!(
        cfg.semantic_provider.as_deref(),
        Some("anthropic"),
        "env should override file"
    );
    assert_eq!(cfg.semantic_model.as_deref(), Some("env-model"));
    assert_eq!(cfg.semantic_api_key.as_deref(), Some("env-key"));
    assert_eq!(
        cfg.semantic_base_url.as_deref(),
        Some("https://env.example.com")
    );
}

// ---------------------------------------------------------------------------
// C. Embedding config from unified
// ---------------------------------------------------------------------------

#[test]
fn embedding_from_unified_reads_values() {
    let settings = UnifiedSettings {
        stores: Some(json!({
            "embedding_provider": "openai",
            "embedding_model": "text-embedding-ada-002",
            "embedding_api_key": "sk-embed"
        })),
        ..Default::default()
    };
    let cfg = resolve_embedding_from_unified(&settings, no_env);

    assert_eq!(cfg.embedding_provider.as_deref(), Some("openai"));
    assert_eq!(
        cfg.embedding_model.as_deref(),
        Some("text-embedding-ada-002")
    );
    assert_eq!(cfg.embedding_api_key.as_deref(), Some("sk-embed"));
}

#[test]
fn embedding_from_unified_defaults_provider_to_local() {
    let settings = UnifiedSettings::default();
    let cfg = resolve_embedding_from_unified(&settings, no_env);

    assert_eq!(
        cfg.embedding_provider.as_deref(),
        Some("local"),
        "default embedding provider should be 'local'"
    );
}

// ---------------------------------------------------------------------------
// D. Watch config from unified (JSON only, no TOML)
// ---------------------------------------------------------------------------

#[test]
fn watch_from_unified_reads_json_values() {
    let settings = UnifiedSettings {
        watch: Some(json!({
            "watch_debounce_ms": 1000,
            "watch_poll_fallback_ms": 5000
        })),
        ..Default::default()
    };
    let cfg = resolve_watch_from_unified(&settings, no_env);

    assert_eq!(cfg.watch_debounce_ms, 1000);
    assert_eq!(cfg.watch_poll_fallback_ms, 5000);
}

#[test]
fn watch_from_unified_env_overrides() {
    let settings = UnifiedSettings {
        watch: Some(json!({
            "watch_debounce_ms": 1000,
            "watch_poll_fallback_ms": 5000
        })),
        ..Default::default()
    };
    let cfg = resolve_watch_from_unified(&settings, |key| match key {
        k if k == ENV_WATCH_DEBOUNCE_MS => Some("200".into()),
        k if k == ENV_WATCH_POLL_FALLBACK_MS => Some("3000".into()),
        _ => None,
    });

    assert_eq!(cfg.watch_debounce_ms, 200, "env should override file");
    assert_eq!(cfg.watch_poll_fallback_ms, 3000);
}

#[test]
fn watch_from_unified_applies_defaults() {
    let settings = UnifiedSettings::default(); // no watch block
    let cfg = resolve_watch_from_unified(&settings, no_env);

    assert_eq!(cfg.watch_debounce_ms, 500, "default debounce is 500ms");
    assert_eq!(
        cfg.watch_poll_fallback_ms, 2000,
        "default poll fallback is 2000ms"
    );
}

// ---------------------------------------------------------------------------
// E. Provider config from unified
// ---------------------------------------------------------------------------

#[test]
fn provider_from_unified_reads_knowledge_block() {
    let settings = UnifiedSettings {
        knowledge: Some(json!({
            "providers": {
                "github": { "token": "gh-token-123" },
                "atlassian": {
                    "site_url": "https://my.atlassian.net",
                    "email": "user@example.com",
                    "token": "atl-token"
                }
            }
        })),
        ..Default::default()
    };
    let cfg = resolve_provider_from_unified(&settings, no_env).unwrap();

    let gh = cfg.github.expect("github should be present");
    assert_eq!(gh.token, "gh-token-123");
    let atl = cfg.atlassian.expect("atlassian should be present");
    assert_eq!(atl.site_url, "https://my.atlassian.net");
    assert_eq!(atl.email, "user@example.com");
    assert_eq!(atl.token, "atl-token");
}

#[test]
fn provider_from_unified_resolves_env_indirection() {
    let settings = UnifiedSettings {
        knowledge: Some(json!({
            "providers": {
                "github": { "token": "${MY_GH_TOKEN}" }
            }
        })),
        ..Default::default()
    };
    let cfg = resolve_provider_from_unified(&settings, |key| match key {
        "MY_GH_TOKEN" => Some("resolved-token".into()),
        _ => None,
    })
    .unwrap();

    let gh = cfg.github.expect("github should be present");
    assert_eq!(gh.token, "resolved-token");
}

// ---------------------------------------------------------------------------
// F. Dashboard config from unified
// ---------------------------------------------------------------------------

#[test]
fn dashboard_from_unified_reads_flag() {
    let settings = UnifiedSettings {
        dashboard: Some(json!({ "use_bitloops_local": true })),
        ..Default::default()
    };
    let cfg = resolve_dashboard_from_unified(&settings);
    assert_eq!(cfg.use_bitloops_local, Some(true));
}

#[test]
fn dashboard_from_unified_defaults_when_absent() {
    let settings = UnifiedSettings::default();
    let cfg = resolve_dashboard_from_unified(&settings);
    assert_eq!(cfg.use_bitloops_local, None);
}

// ---------------------------------------------------------------------------
// G. Cross-layer integration: watch replaces TOML with JSON layers
// ---------------------------------------------------------------------------

#[test]
fn watch_from_unified_merges_across_json_layers() {
    let global = UnifiedSettings {
        watch: Some(json!({ "watch_debounce_ms": 1000 })),
        ..Default::default()
    };
    let project = UnifiedSettings {
        watch: Some(json!({ "watch_poll_fallback_ms": 4000 })),
        ..Default::default()
    };
    let merged = merge_layers(&[global, project]);
    let cfg = resolve_watch_from_unified(&merged, no_env);

    assert_eq!(
        cfg.watch_debounce_ms, 1000,
        "debounce from global should propagate via JSON merge"
    );
    assert_eq!(
        cfg.watch_poll_fallback_ms, 4000,
        "poll fallback from project should override default"
    );
}

// ---------------------------------------------------------------------------
// H. Provider-less store backend from unified config (spec §5.1, CLI-1480)
//
// These tests assert the target API where provider enums are removed and
// backend availability is derived from connection-string presence.
// They MUST fail against the current codebase (proving the gap).
// ---------------------------------------------------------------------------

#[test]
fn store_backend_from_unified_has_postgres_when_dsn_present() {
    let settings = UnifiedSettings {
        stores: Some(json!({
            "relational": { "postgres_dsn": "postgres://localhost/db" },
            "events": { "clickhouse_url": "http://ch:8123" }
        })),
        ..Default::default()
    };
    let tmp = tempfile::tempdir().unwrap();
    let cfg = resolve_store_backend_from_unified(&settings, tmp.path()).unwrap();

    assert!(
        cfg.relational.has_postgres(),
        "postgres_dsn present → has_postgres"
    );
    assert!(
        cfg.events.has_clickhouse(),
        "clickhouse_url present → has_clickhouse"
    );
}

#[test]
fn store_backend_from_unified_defaults_have_no_remote_capabilities() {
    let settings = UnifiedSettings::default();
    let tmp = tempfile::tempdir().unwrap();
    let cfg = resolve_store_backend_from_unified(&settings, tmp.path()).unwrap();

    assert!(
        !cfg.relational.has_postgres(),
        "default should not have postgres"
    );
    assert!(
        !cfg.events.has_clickhouse(),
        "default should not have clickhouse"
    );
    assert!(
        !cfg.blobs.has_remote(),
        "default should not have remote blob"
    );
}
