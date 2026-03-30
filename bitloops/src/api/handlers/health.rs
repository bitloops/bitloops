use axum::{Json, extract::State};

use super::super::dto::{ApiBackendHealthDto, ApiDbHealthResponse};
use super::super::{DashboardState, db};

#[utoipa::path(
    get,
    path = "/api/db/health",
    responses((status = 200, description = "Live database backend health", body = ApiDbHealthResponse))
)]
pub(crate) async fn handle_api_db_health(
    State(state): State<DashboardState>,
) -> Json<ApiDbHealthResponse> {
    let health = state.db.health_check().await;

    Json(ApiDbHealthResponse {
        relational: map_backend_health(health.relational),
        events: map_backend_health(health.events),
        postgres: map_backend_health(health.postgres),
        clickhouse: map_backend_health(health.clickhouse),
    })
}

fn map_backend_health(health: db::BackendHealth) -> ApiBackendHealthDto {
    ApiBackendHealthDto {
        status: health.status_label().to_string(),
        detail: health.detail,
    }
}
