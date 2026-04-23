#[path = "analytics/cache.rs"]
mod cache;
#[path = "analytics/derived_tables.rs"]
mod derived_tables;
#[path = "analytics/format.rs"]
mod format;
#[path = "analytics/query.rs"]
mod query;
#[path = "analytics/row_access.rs"]
mod row_access;
#[path = "analytics/scope.rs"]
mod scope;
#[path = "analytics/source_tables.rs"]
mod source_tables;
#[path = "analytics/specs.rs"]
mod specs;
#[path = "analytics/sql_validation.rs"]
mod sql_validation;
#[cfg(test)]
#[path = "analytics/tests.rs"]
mod tests;
#[path = "analytics/types.rs"]
mod types;

use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::Value;

use self::cache::refresh_analytics_cache;
pub(crate) use self::format::format_analytics_sql_result_table;
use self::query::run_analytics_query;
use self::scope::resolve_analytics_scope;
use self::sql_validation::validate_analytics_sql;
use self::types::ANALYTICS_MAX_ROWS;
pub(crate) use self::types::{AnalyticsRepoScope, AnalyticsSqlResult};

use super::DevqlConfig;
use crate::config::resolve_store_backend_config_for_repo;

pub(crate) async fn execute_analytics_sql(
    cfg: &DevqlConfig,
    scope: AnalyticsRepoScope,
    sql: &str,
) -> Result<AnalyticsSqlResult> {
    let started = Instant::now();
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving analytics backend configuration")?;
    let repositories = resolve_analytics_scope(cfg, &backends, scope).await?;
    let repo_ids = repositories
        .iter()
        .map(|repository| repository.repo_id.clone())
        .collect::<Vec<_>>();
    let validated_sql = validate_analytics_sql(sql)?;

    refresh_analytics_cache(cfg, &backends, &repositories).await?;
    let query_result = run_analytics_query(cfg, &repo_ids, &validated_sql).await?;

    let mut warnings = Vec::new();
    if query_result.truncated {
        warnings.push(format!("Result truncated to {ANALYTICS_MAX_ROWS} rows."));
    }

    Ok(AnalyticsSqlResult {
        row_count: query_result.rows.len(),
        rows: Value::Array(query_result.rows),
        columns: query_result.columns,
        truncated: query_result.truncated,
        duration_ms: started.elapsed().as_millis() as u64,
        repo_ids,
        warnings,
    })
}
