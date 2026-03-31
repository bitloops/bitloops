use serde_json::json;
use std::path::Path;

use super::unified_config::{
<<<<<<< Updated upstream
    UnifiedSettings, merge_layers, resolve_dashboard_from_unified,
    resolve_embedding_capability_from_unified, resolve_embedding_from_unified,
    resolve_embeddings_from_unified, resolve_provider_from_unified,
    resolve_semantic_clones_from_unified, resolve_semantic_from_unified,
=======
<<<<<<< Updated upstream
    UnifiedSettings, merge_layers, resolve_dashboard_from_unified, resolve_embedding_from_unified,
    resolve_provider_from_unified, resolve_semantic_from_unified,
>>>>>>> Stashed changes
    resolve_store_backend_from_unified, resolve_watch_from_unified,
=======
    UnifiedSettings, merge_layers, resolve_dashboard_from_unified,
    resolve_embedding_capability_from_unified, resolve_embeddings_from_unified,
    resolve_provider_from_unified, resolve_semantic_clones_from_unified,
    resolve_semantic_from_unified, resolve_store_backend_from_unified,
    resolve_watch_from_unified,
>>>>>>> Stashed changes
};
use super::{
    DashboardLocalDashboardConfig, ENV_SEMANTIC_API_KEY, ENV_SEMANTIC_BASE_URL,
    ENV_SEMANTIC_MODEL, ENV_SEMANTIC_PROVIDER, ENV_WATCH_DEBOUNCE_MS,
    ENV_WATCH_POLL_FALLBACK_MS, SemanticCloneEmbeddingMode, SemanticSummaryMode,
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
<<<<<<< Updated upstream
fn embedding_from_unified_ignores_legacy_values() {
=======
<<<<<<< Updated upstream
fn embedding_from_unified_reads_values() {
=======
fn embedding_capability_from_unified_requires_explicit_profile_selection() {
>>>>>>> Stashed changes
>>>>>>> Stashed changes
    let settings = UnifiedSettings {
        embeddings: Some(json!({
            "profiles": {
                "local": {
                    "kind": "local_fastembed",
                    "model": "jinaai/jina-embeddings-v2-base-code"
                }
            }
        })),
        ..Default::default()
    };
    let capability =
        resolve_embedding_capability_from_unified(&settings, Path::new("/config"), no_env);

<<<<<<< Updated upstream
    assert_eq!(cfg.embedding_provider, None);
    assert_eq!(cfg.embedding_model, None);
    assert_eq!(cfg.embedding_api_key, None);
}

#[test]
fn embedding_from_unified_defaults_to_disabled() {
    let settings = UnifiedSettings::default();
    let cfg = resolve_embedding_from_unified(&settings, Path::new("/config"), no_env);

    assert_eq!(cfg.embedding_provider, None);
    assert_eq!(cfg.embedding_model, None);
    assert_eq!(cfg.embedding_api_key, None);
=======
<<<<<<< Updated upstream
    assert_eq!(cfg.embedding_provider.as_deref(), Some("openai"));
    assert_eq!(
        cfg.embedding_model.as_deref(),
        Some("text-embedding-ada-002")
    );
    assert_eq!(cfg.embedding_api_key.as_deref(), Some("sk-embed"));
}

#[test]
fn embedding_from_unified_defaults_provider_to_local() {
=======
    assert_eq!(capability.semantic_clones.embedding_profile, None);
    assert!(capability.embeddings.profiles.contains_key("local"));
    assert!(capability.embeddings.warnings.is_empty());
}

#[test]
fn embeddings_from_unified_defaults_to_disabled() {
>>>>>>> Stashed changes
    let settings = UnifiedSettings::default();
    let embeddings = resolve_embeddings_from_unified(&settings, Path::new("/config"), no_env);

<<<<<<< Updated upstream
=======
    assert_eq!(embeddings.runtime.command, "bitloops-embeddings");
    assert!(embeddings.profiles.is_empty());
    assert!(embeddings.warnings.is_empty());
>>>>>>> Stashed changes
}

#[test]
fn semantic_clones_and_embeddings_from_unified_read_profile_sections() {
    let settings = UnifiedSettings {
        semantic_clones: Some(json!({
            "summary_mode": "auto",
            "embedding_mode": "semantic_aware_once",
            "embedding_profile": "local"
        })),
        embeddings: Some(json!({
            "runtime": {
                "command": "bitloops-embeddings",
                "args": ["--verbose"]
            },
            "profiles": {
                "local": {
                    "kind": "local_fastembed",
                    "model": "jinaai/jina-embeddings-v2-base-code",
                    "cache_dir": ".cache/embeddings"
                }
            }
        })),
        ..Default::default()
    };

    let semantic_clones = resolve_semantic_clones_from_unified(&settings, no_env);
    assert_eq!(semantic_clones.summary_mode, SemanticSummaryMode::Auto);
<<<<<<< Updated upstream
=======
>>>>>>> Stashed changes
>>>>>>> Stashed changes
    assert_eq!(
        semantic_clones.embedding_mode,
        SemanticCloneEmbeddingMode::SemanticAwareOnce
    );
<<<<<<< Updated upstream
=======
<<<<<<< Updated upstream
=======
>>>>>>> Stashed changes
    assert_eq!(semantic_clones.embedding_profile.as_deref(), Some("local"));

    let embeddings = resolve_embeddings_from_unified(&settings, Path::new("/config"), no_env);
    assert_eq!(embeddings.runtime.command, "bitloops-embeddings");
    assert_eq!(embeddings.runtime.args, vec!["--verbose".to_string()]);
    let profile = embeddings
        .profiles
        .get("local")
        .expect("local embedding profile");
    assert_eq!(profile.kind, "local_fastembed");
    assert_eq!(
        profile.cache_dir.as_deref(),
        Some(Path::new("/config/.cache/embeddings"))
    );
}

#[test]
<<<<<<< Updated upstream
fn embedding_capability_from_unified_does_not_synthesize_legacy_profile() {
    let settings = UnifiedSettings {
        stores: Some(json!({
            "embedding_provider": "openai",
            "embedding_model": "text-embedding-3-large",
            "embedding_api_key": "legacy-key"
=======
fn embedding_capability_from_unified_does_not_activate_from_unrelated_store_settings() {
    let settings = UnifiedSettings {
        stores: Some(json!({
            "relational": {
                "sqlite_path": "data/devql.sqlite"
            }
>>>>>>> Stashed changes
        })),
        ..Default::default()
    };

    let capability =
        resolve_embedding_capability_from_unified(&settings, Path::new("/config"), no_env);
    assert_eq!(capability.semantic_clones.embedding_profile, None);
    assert!(capability.embeddings.profiles.is_empty());
    assert!(capability.embeddings.warnings.is_empty());
}

#[test]
fn semantic_clones_from_unified_reads_mode_fields() {
    let settings = UnifiedSettings {
        semantic_clones: Some(json!({
            "summary_mode": "off",
            "embedding_mode": "refresh_on_upgrade",
        })),
        ..Default::default()
    };

    let semantic_clones = resolve_semantic_clones_from_unified(&settings, no_env);
    assert_eq!(semantic_clones.summary_mode, SemanticSummaryMode::Off);
    assert_eq!(
        semantic_clones.embedding_mode,
        SemanticCloneEmbeddingMode::RefreshOnUpgrade
    );
    assert_eq!(semantic_clones.embedding_profile, None);
<<<<<<< Updated upstream
=======
>>>>>>> Stashed changes
>>>>>>> Stashed changes
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
fn dashboard_from_unified_reads_local_dashboard_flags() {
    let settings = UnifiedSettings {
        dashboard: Some(json!({
            "local_dashboard": {
                "tls": true
            }
        })),
        ..Default::default()
    };
    let cfg = resolve_dashboard_from_unified(&settings, Path::new("/config"));
    assert_eq!(
        cfg.local_dashboard,
        Some(DashboardLocalDashboardConfig { tls: Some(true) })
    );
}

#[test]
fn dashboard_from_unified_defaults_when_absent() {
    let settings = UnifiedSettings::default();
    let cfg = resolve_dashboard_from_unified(&settings, Path::new("/config"));
    assert_eq!(cfg.local_dashboard, None);
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
