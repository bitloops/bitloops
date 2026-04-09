use serde_json::Value;

use crate::config::{EmbeddingsConfig, ProviderConfig, SemanticClonesConfig, StoreBackendConfig};

pub(super) fn build_capability_config_root(
    backends: &StoreBackendConfig,
    providers: &ProviderConfig,
    semantic_clones: &SemanticClonesConfig,
    embeddings: &EmbeddingsConfig,
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
            "summary_profile": semantic_clones.summary_profile,
            "embedding_mode": semantic_clones.embedding_mode,
            "embedding_profile": semantic_clones.embedding_profile,
            "ann_neighbors": semantic_clones.ann_neighbors,
            "enrichment_workers": semantic_clones.enrichment_workers,
        },
        "embeddings": {
            "runtime": {
                "command": embeddings.runtime.command,
                "args": embeddings.runtime.args,
                "startup_timeout_secs": embeddings.runtime.startup_timeout_secs,
                "request_timeout_secs": embeddings.runtime.request_timeout_secs,
            },
            "profiles": embeddings.profiles.iter().map(|(name, profile)| (
                name.clone(),
                serde_json::json!({
                    "kind": profile.kind,
                    "model": profile.model,
                    "base_url": profile.base_url,
                    "cache_dir": profile.cache_dir,
                })
            )).collect::<serde_json::Map<_, _>>(),
            "warnings": embeddings.warnings,
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
