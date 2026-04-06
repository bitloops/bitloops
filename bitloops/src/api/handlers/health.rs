use axum::{Json, extract::State};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

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
    let started = Instant::now();
    let health = state.db.health_check().await;
    let response = Json(ApiDbHealthResponse {
        relational: map_backend_health(health.relational),
        events: map_backend_health(health.events),
        postgres: map_backend_health(health.postgres),
        clickhouse: map_backend_health(health.clickhouse),
    });

    let mut properties = HashMap::new();
    properties.insert("http_method".to_string(), Value::String("GET".to_string()));
    properties.insert(
        "status_code_class".to_string(),
        Value::String("2xx".to_string()),
    );
    super::super::track_repo_action(
        &state.repo_root,
        crate::telemetry::analytics::ActionDescriptor {
            event: "bitloops dashboard api db-health".to_string(),
            surface: "dashboard",
            properties,
        },
        true,
        started.elapsed(),
    );

    response
}

fn map_backend_health(health: db::BackendHealth) -> ApiBackendHealthDto {
    ApiBackendHealthDto {
        status: health.status_label().to_string(),
        detail: health.detail,
    }
}
