use axum::Json;
use utoipa::OpenApi;

use super::super::dto::{ApiError, ApiRootResponse, DashboardApiDoc};

#[utoipa::path(
    get,
    path = "/api",
    responses((status = 200, description = "Dashboard API root", body = ApiRootResponse))
)]
pub(crate) async fn handle_api_root() -> Json<ApiRootResponse> {
    Json(ApiRootResponse {
        name: "bitloops-dashboard-api".to_string(),
        openapi: "/api/openapi.json".to_string(),
    })
}

#[utoipa::path(
    get,
    path = "/api/openapi.json",
    responses((status = 200, description = "Generated OpenAPI document"))
)]
pub(crate) async fn handle_api_openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(DashboardApiDoc::openapi())
}

pub(crate) async fn handle_api_not_found() -> ApiError {
    ApiError::not_found("route not found")
}
