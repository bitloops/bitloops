use serde_json::Value;

use crate::config::{ProviderConfig, StoreBackendConfig};

pub(super) fn build_capability_config_root(
    backends: &StoreBackendConfig,
    providers: &ProviderConfig,
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
