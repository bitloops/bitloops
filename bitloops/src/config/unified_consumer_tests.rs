use serde_json::json;
use std::path::Path;

use super::unified_config::{
    UnifiedSettings, merge_layers, resolve_context_guidance_from_unified,
    resolve_dashboard_from_unified, resolve_embedding_capability_from_unified,
    resolve_embeddings_from_unified, resolve_inference_capability_from_unified,
    resolve_provider_from_unified, resolve_semantic_clones_from_unified,
    resolve_store_backend_from_unified, resolve_watch_from_unified,
};
use super::{
    ContextGuidanceInferenceBindings, DEFAULT_SEMANTIC_CLONES_CLONE_REBUILD_WORKERS,
    DEFAULT_SEMANTIC_CLONES_EMBEDDING_WORKERS, DEFAULT_SEMANTIC_CLONES_ENRICHMENT_WORKERS,
    DEFAULT_SEMANTIC_CLONES_SUMMARY_WORKERS, DashboardLocalDashboardConfig, ENV_WATCH_DEBOUNCE_MS,
    ENV_WATCH_POLL_FALLBACK_MS, InferenceTask, SemanticCloneEmbeddingMode,
    SemanticClonesInferenceBindings, SemanticSummaryMode,
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
    let settings = UnifiedSettings::default();
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
// B. Semantic clones + inference config from unified
// ---------------------------------------------------------------------------

#[test]
fn semantic_clones_and_inference_from_unified_read_slot_bindings() {
    let settings = UnifiedSettings {
        semantic_clones: Some(json!({
            "summary_mode": "auto",
            "embedding_mode": "semantic_aware_once",
            "summary_workers": 3,
            "embedding_workers": 12,
            "clone_rebuild_workers": 2,
            "enrichment_workers": 12,
            "inference": {
                "summary_generation": "summary_llm",
                "code_embeddings": "local_code",
                "summary_embeddings": "local_summary"
            }
        })),
        inference: Some(json!({
            "runtimes": {
                "bitloops_local_embeddings": {
                    "command": "bitloops-local-embeddings",
                    "args": ["--verbose"]
                },
                "bitloops_inference": {
                    "command": "bitloops-inference"
                }
            },
            "profiles": {
                "local_code": {
                    "task": "embeddings",
                    "driver": "bitloops_embeddings_ipc",
                    "runtime": "bitloops_local_embeddings",
                    "model": "bge-m3",
                    "cache_dir": ".cache/embeddings"
                },
                "local_summary": {
                    "task": "embeddings",
                    "driver": "bitloops_embeddings_ipc",
                    "runtime": "bitloops_local_embeddings",
                    "model": "bge-m3"
                },
                "summary_llm": {
                    "task": "text_generation",
                    "driver": "ollama_chat",
                    "runtime": "bitloops_inference",
                    "model": "gpt-5.4-mini",
                    "base_url": "https://api.openai.com/v1",
                    "temperature": "0.1",
                    "max_output_tokens": 200
                }
            }
        })),
        ..Default::default()
    };
    let semantic_clones = resolve_semantic_clones_from_unified(&settings, no_env);
    let inference = resolve_embeddings_from_unified(&settings, Path::new("/config"), no_env);
    let capability =
        resolve_embedding_capability_from_unified(&settings, Path::new("/config"), no_env);

    assert_eq!(semantic_clones.summary_mode, SemanticSummaryMode::Auto);
    assert_eq!(
        semantic_clones.embedding_mode,
        SemanticCloneEmbeddingMode::SemanticAwareOnce
    );
    assert_eq!(
        semantic_clones.inference,
        SemanticClonesInferenceBindings {
            summary_generation: Some("summary_llm".to_string()),
            code_embeddings: Some("local_code".to_string()),
            summary_embeddings: Some("local_summary".to_string()),
        }
    );
    assert_eq!(semantic_clones.ann_neighbors, 5);
    assert_eq!(semantic_clones.summary_workers, 3);
    assert_eq!(semantic_clones.embedding_workers, 12);
    assert_eq!(semantic_clones.clone_rebuild_workers, 2);
    assert_eq!(semantic_clones.enrichment_workers, 12);
    assert!(inference.warnings.is_empty());
    assert_eq!(
        inference
            .runtimes
            .get("bitloops_local_embeddings")
            .expect("runtime")
            .args,
        vec!["--verbose".to_string()]
    );
    let code_profile = inference.profiles.get("local_code").expect("code profile");
    assert_eq!(code_profile.task, InferenceTask::Embeddings);
    assert_eq!(code_profile.driver, "bitloops_embeddings_ipc");
    assert_eq!(
        code_profile.runtime.as_deref(),
        Some("bitloops_local_embeddings")
    );
    assert_eq!(code_profile.model.as_deref(), Some("bge-m3"));
    assert_eq!(
        code_profile.cache_dir.as_deref(),
        Some(Path::new("/config/.cache/embeddings"))
    );
    let llm_profile = inference.profiles.get("summary_llm").expect("llm profile");
    assert_eq!(llm_profile.task, InferenceTask::TextGeneration);
    assert_eq!(llm_profile.driver, "ollama_chat");
    assert_eq!(llm_profile.runtime.as_deref(), Some("bitloops_inference"));
    assert_eq!(llm_profile.model.as_deref(), Some("gpt-5.4-mini"));
    assert_eq!(
        llm_profile.base_url.as_deref(),
        Some("https://api.openai.com/v1")
    );
    assert_eq!(llm_profile.temperature.as_deref(), Some("0.1"));
    assert_eq!(llm_profile.max_output_tokens, Some(200));
    assert_eq!(capability.semantic_clones, semantic_clones);
    assert_eq!(capability.inference, inference);
}

#[test]
fn context_guidance_and_inference_from_unified_read_slot_binding() {
    let settings = UnifiedSettings {
        context_guidance: Some(json!({
            "inference": {
                "guidance_generation": "guidance_local"
            }
        })),
        inference: Some(json!({
            "profiles": {
                "guidance_local": {
                    "task": "text_generation",
                    "driver": "bitloops_platform_chat",
                    "runtime": "bitloops_inference",
                    "model": "guidance-model",
                    "temperature": "0.1",
                    "max_output_tokens": 4096
                }
            }
        })),
        ..Default::default()
    };

    let context_guidance = resolve_context_guidance_from_unified(&settings, no_env);
    let capability =
        resolve_inference_capability_from_unified(&settings, Path::new("/config"), no_env);

    assert_eq!(
        context_guidance.inference,
        ContextGuidanceInferenceBindings {
            guidance_generation: Some("guidance_local".to_string()),
        }
    );
    assert_eq!(capability.context_guidance, context_guidance);
    assert_eq!(
        capability
            .inference
            .profiles
            .get("guidance_local")
            .expect("guidance profile")
            .task,
        InferenceTask::TextGeneration
    );
}

#[test]
fn embedding_capability_from_unified_does_not_activate_from_unrelated_store_settings() {
    let settings = UnifiedSettings {
        stores: Some(json!({
            "relational": {
                "sqlite_path": "data/devql.sqlite"
            }
        })),
        ..Default::default()
    };

    let capability =
        resolve_embedding_capability_from_unified(&settings, Path::new("/config"), no_env);
    assert_eq!(capability.semantic_clones.inference.code_embeddings, None);
    assert!(capability.inference.profiles.is_empty());
    assert!(capability.inference.warnings.is_empty());
}

#[test]
fn semantic_clones_from_unified_reads_mode_fields() {
    let settings = UnifiedSettings {
        semantic_clones: Some(json!({
            "summary_mode": "off",
            "embedding_mode": "refresh_on_upgrade",
            "ann_neighbors": 17,
        })),
        ..Default::default()
    };

    let semantic_clones = resolve_semantic_clones_from_unified(&settings, no_env);
    assert_eq!(semantic_clones.summary_mode, SemanticSummaryMode::Off);
    assert_eq!(semantic_clones.inference.summary_generation, None);
    assert_eq!(
        semantic_clones.embedding_mode,
        SemanticCloneEmbeddingMode::RefreshOnUpgrade
    );
    assert_eq!(semantic_clones.inference.code_embeddings, None);
    assert_eq!(semantic_clones.ann_neighbors, 17);
    assert_eq!(
        semantic_clones.summary_workers,
        DEFAULT_SEMANTIC_CLONES_SUMMARY_WORKERS
    );
    assert_eq!(
        semantic_clones.embedding_workers,
        DEFAULT_SEMANTIC_CLONES_EMBEDDING_WORKERS
    );
    assert_eq!(
        semantic_clones.clone_rebuild_workers,
        DEFAULT_SEMANTIC_CLONES_CLONE_REBUILD_WORKERS
    );
    assert_eq!(
        semantic_clones.enrichment_workers,
        DEFAULT_SEMANTIC_CLONES_ENRICHMENT_WORKERS
    );
}

#[test]
fn semantic_clones_from_unified_clamps_ann_neighbors_from_env() {
    let settings = UnifiedSettings::default();
    let semantic_clones = resolve_semantic_clones_from_unified(&settings, |key| match key {
        "BITLOOPS_SEMANTIC_CLONES_ANN_NEIGHBORS" => Some("999".to_string()),
        _ => None,
    });
    assert_eq!(semantic_clones.ann_neighbors, 50);
    assert_eq!(
        semantic_clones.summary_workers,
        DEFAULT_SEMANTIC_CLONES_SUMMARY_WORKERS
    );
    assert_eq!(
        semantic_clones.embedding_workers,
        DEFAULT_SEMANTIC_CLONES_EMBEDDING_WORKERS
    );
    assert_eq!(
        semantic_clones.clone_rebuild_workers,
        DEFAULT_SEMANTIC_CLONES_CLONE_REBUILD_WORKERS
    );
    assert_eq!(
        semantic_clones.enrichment_workers,
        DEFAULT_SEMANTIC_CLONES_ENRICHMENT_WORKERS
    );
}

#[test]
fn semantic_clones_from_unified_reads_enrichment_workers_from_env() {
    let settings = UnifiedSettings::default();
    let semantic_clones = resolve_semantic_clones_from_unified(&settings, |key| match key {
        "BITLOOPS_SEMANTIC_CLONES_ENRICHMENT_WORKERS" => Some("16".to_string()),
        _ => None,
    });
    assert_eq!(semantic_clones.enrichment_workers, 16);
    assert_eq!(semantic_clones.embedding_workers, 16);
}

#[test]
fn semantic_clones_from_unified_reads_per_pool_workers_from_env() {
    let settings = UnifiedSettings::default();
    let semantic_clones = resolve_semantic_clones_from_unified(&settings, |key| match key {
        "BITLOOPS_SEMANTIC_CLONES_SUMMARY_WORKERS" => Some("2".to_string()),
        "BITLOOPS_SEMANTIC_CLONES_EMBEDDING_WORKERS" => Some("5".to_string()),
        "BITLOOPS_SEMANTIC_CLONES_CLONE_REBUILD_WORKERS" => Some("3".to_string()),
        _ => None,
    });
    assert_eq!(semantic_clones.summary_workers, 2);
    assert_eq!(semantic_clones.embedding_workers, 5);
    assert_eq!(semantic_clones.clone_rebuild_workers, 3);
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

    assert_eq!(cfg.watch_debounce_ms, 200);
    assert_eq!(cfg.watch_poll_fallback_ms, 3000);
}

#[test]
fn watch_from_unified_applies_defaults() {
    let settings = UnifiedSettings::default();
    let cfg = resolve_watch_from_unified(&settings, no_env);

    assert_eq!(cfg.watch_debounce_ms, 500);
    assert_eq!(cfg.watch_poll_fallback_ms, 2000);
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

    assert_eq!(cfg.watch_debounce_ms, 1000);
    assert_eq!(cfg.watch_poll_fallback_ms, 4000);
}

// ---------------------------------------------------------------------------
// H. Provider-less store backend from unified config
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

    assert!(cfg.relational.has_postgres());
    assert!(cfg.events.has_clickhouse());
}

#[test]
fn store_backend_from_unified_defaults_have_no_remote_capabilities() {
    let settings = UnifiedSettings::default();
    let tmp = tempfile::tempdir().unwrap();
    let cfg = resolve_store_backend_from_unified(&settings, tmp.path()).unwrap();

    assert!(!cfg.relational.has_postgres());
    assert!(!cfg.events.has_clickhouse());
    assert!(!cfg.blobs.has_remote());
}
