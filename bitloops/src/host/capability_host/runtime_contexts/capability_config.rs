use serde_json::Value;

use crate::config::{InferenceConfig, ProviderConfig, SemanticClonesConfig, StoreBackendConfig};

pub(super) fn build_capability_config_root(
    backends: &StoreBackendConfig,
    providers: &ProviderConfig,
    semantic_clones: &SemanticClonesConfig,
    inference: &InferenceConfig,
) -> Value {
    serde_json::json!({
        "knowledge": {
            "providers": {
                "github": providers.github.as_ref().map(|_| serde_json::json!({ "configured": true })),
                "jira": providers.jira.as_ref().map(|cfg| serde_json::json!({ "site_url": cfg.site_url })),
                "confluence": providers.confluence.as_ref().map(|cfg| serde_json::json!({ "site_url": cfg.site_url })),
                "atlassian": providers.atlassian.as_ref().map(|cfg| serde_json::json!({ "site_url": cfg.site_url })),
            },
            "backends": {
                "relational": if backends.relational.has_postgres() { "postgres" } else { "sqlite" },
                "events": if backends.events.has_clickhouse() { "clickhouse" } else { "duckdb" },
            }
        },
        "semantic_clones": {
            "summary_mode": semantic_clones.summary_mode,
            "embedding_mode": semantic_clones.embedding_mode,
            "ann_neighbors": semantic_clones.ann_neighbors,
            "enrichment_workers": semantic_clones.enrichment_workers,
            "inference": {
                "summary_generation": semantic_clones.inference.summary_generation,
                "code_embeddings": semantic_clones.inference.code_embeddings,
                "summary_embeddings": semantic_clones.inference.summary_embeddings,
            },
        },
        "inference": {
            "runtimes": inference.runtimes.iter().map(|(name, runtime)| (
                name.clone(),
                serde_json::json!({
                    "command": runtime.command,
                    "args": runtime.args,
                    "startup_timeout_secs": runtime.startup_timeout_secs,
                    "request_timeout_secs": runtime.request_timeout_secs,
                })
            )).collect::<serde_json::Map<_, _>>(),
            "profiles": inference.profiles.iter().map(|(name, profile)| (
                name.clone(),
                serde_json::json!({
                    "task": profile.task,
                    "driver": profile.driver,
                    "runtime": profile.runtime,
                    "model": profile.model,
                    "api_key": profile.api_key.as_ref().map(|_| "<configured>"),
                    "base_url": profile.base_url,
                    "cache_dir": profile.cache_dir,
                })
            )).collect::<serde_json::Map<_, _>>(),
            "warnings": inference.warnings,
        },
        "host": {
            "invocation": {
                "stage_timeout_secs": 120,
                "ingester_timeout_secs": 300,
                "subquery_timeout_secs": 60
            },
            "cross_pack_access": []
        }
    })
}
