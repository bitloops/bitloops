use axum::http::StatusCode;

use crate::api::dashboard_types::{
    DashboardAnalyticsColumn, DashboardAnalyticsSqlInput, DashboardAnalyticsSqlResult,
};
use crate::api::{ApiError, DashboardState};
use crate::host::devql::{
    AnalyticsRepoScope, DevqlConfig, execute_analytics_sql, resolve_repo_identity,
};

pub(in crate::api) async fn load_dashboard_analytics_sql(
    state: &DashboardState,
    input: DashboardAnalyticsSqlInput,
) -> std::result::Result<DashboardAnalyticsSqlResult, ApiError> {
    let repo_identity = resolve_repo_identity(&state.repo_root).map_err(|err| {
        ApiError::internal(format!(
            "failed to resolve dashboard analytics repository scope: {err:#}"
        ))
    })?;
    let cfg = DevqlConfig::from_roots(
        state.config_root.clone(),
        state.repo_root.clone(),
        repo_identity,
    )
    .map_err(|err| {
        ApiError::internal(format!(
            "failed to build dashboard analytics configuration: {err:#}"
        ))
    })?;

    let scope = match input.repo_ids {
        Some(repo_ids) if !repo_ids.is_empty() => AnalyticsRepoScope::Explicit(repo_ids),
        _ => AnalyticsRepoScope::AllKnown,
    };
    let result = execute_analytics_sql(&cfg, scope, &input.sql)
        .await
        .map_err(map_analytics_error)?;

    Ok(DashboardAnalyticsSqlResult {
        columns: result
            .columns
            .into_iter()
            .map(|column| DashboardAnalyticsColumn {
                name: column.name,
                logical_type: column.logical_type,
            })
            .collect(),
        rows: async_graphql::types::Json(result.rows),
        row_count: i32::try_from(result.row_count).unwrap_or(i32::MAX),
        truncated: result.truncated,
        duration_ms: i32::try_from(result.duration_ms).unwrap_or(i32::MAX),
        repo_ids: result.repo_ids,
        warnings: result.warnings,
    })
}

fn map_analytics_error(error: anyhow::Error) -> ApiError {
    let message = format!("{error:#}");
    if message.contains("analytics SQL")
        || message.contains("unknown repository")
        || message.contains("ambiguous")
    {
        return ApiError::bad_request(message);
    }
    if message.contains("timed out") {
        return ApiError::with_code(StatusCode::REQUEST_TIMEOUT, "timeout", message);
    }
    ApiError::internal(message)
}
