use std::time::Instant;

use async_graphql_axum::{GraphQLProtocol, GraphQLRequest, GraphQLWebSocket};
use axum::{
    extract::{State, WebSocketUpgrade},
    http::HeaderMap,
    response::{IntoResponse, Response as AxumResponse},
};

use super::roots::RuntimeRequestContext;
use super::schema::build_runtime_schema;
use crate::api::DashboardState;
use crate::devql_transport::parse_repo_root_header;
use crate::graphql::{
    GraphqlActionTelemetry, execute_graphql_request, graphql_error_response,
    graphql_playground_response, graphql_request_signature, track_graphql_action,
    validate_repo_daemon_binding,
};

pub(crate) async fn runtime_graphql_handler(
    State(state): State<DashboardState>,
    headers: HeaderMap,
    request: GraphQLRequest,
) -> AxumResponse {
    let started = Instant::now();
    let request = request.into_inner();
    let signature = graphql_request_signature(&request);
    let repo_root = match parse_repo_root_header(&headers) {
        Ok(repo_root) => repo_root,
        Err(err) => {
            let response = graphql_error_response(err).into_response();
            track_graphql_action(GraphqlActionTelemetry {
                repo_root: state.repo_root.as_path(),
                event: "bitloops devql runtime http",
                scope: "runtime",
                transport: "http",
                request_kind: &signature.0,
                operation_family: &signature.1,
                success: false,
                status: response.status(),
                duration: started.elapsed(),
            });
            return response;
        }
    };
    if let Err(err) = validate_repo_daemon_binding(&headers, &state, repo_root.as_deref()) {
        let response = graphql_error_response(err).into_response();
        track_graphql_action(GraphqlActionTelemetry {
            repo_root: repo_root.as_deref().unwrap_or(state.repo_root.as_path()),
            event: "bitloops devql runtime http",
            scope: "runtime",
            transport: "http",
            request_kind: &signature.0,
            operation_family: &signature.1,
            success: false,
            status: response.status(),
            duration: started.elapsed(),
        });
        return response;
    }
    let request_context = RuntimeRequestContext {
        bound_repo_root: repo_root.clone(),
    };
    let (response, success) = execute_graphql_request(
        state.runtime_graphql_schema(),
        request.data(state.clone()).data(request_context),
        &headers,
    )
    .await;
    let response = response.into_response();
    track_graphql_action(GraphqlActionTelemetry {
        repo_root: state.repo_root.as_path(),
        event: "bitloops devql runtime http",
        scope: "runtime",
        transport: "http",
        request_kind: &signature.0,
        operation_family: &signature.1,
        success,
        status: response.status(),
        duration: started.elapsed(),
    });
    response
}

pub(crate) async fn runtime_graphql_ws_handler(
    State(state): State<DashboardState>,
    protocol: GraphQLProtocol,
    upgrade: WebSocketUpgrade,
    headers: HeaderMap,
) -> impl IntoResponse {
    let started = Instant::now();
    let repo_root = match parse_repo_root_header(&headers) {
        Ok(repo_root) => repo_root,
        Err(err) => return graphql_error_response(err).into_response(),
    };
    if let Err(err) = validate_repo_daemon_binding(&headers, &state, repo_root.as_deref()) {
        return graphql_error_response(err).into_response();
    }
    let schema = build_runtime_schema(
        state.clone(),
        RuntimeRequestContext {
            bound_repo_root: repo_root,
        },
    );
    let response = upgrade
        .protocols(async_graphql::http::ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| GraphQLWebSocket::new(stream, schema, protocol).serve())
        .into_response();
    track_graphql_action(GraphqlActionTelemetry {
        repo_root: state.repo_root.as_path(),
        event: "bitloops devql runtime ws",
        scope: "runtime",
        transport: "ws",
        request_kind: "subscription",
        operation_family: "anonymous",
        success: response.status().is_success()
            || response.status() == axum::http::StatusCode::SWITCHING_PROTOCOLS,
        status: response.status(),
        duration: started.elapsed(),
    });
    response
}

pub(crate) async fn runtime_graphql_playground_handler() -> impl IntoResponse {
    graphql_playground_response(
        "/devql/runtime",
        Some("/devql/runtime/ws"),
        "DevQL Runtime Explorer",
    )
}

pub(crate) async fn runtime_graphql_sdl_handler(
    State(state): State<DashboardState>,
) -> AxumResponse {
    (
        [("content-type", "text/plain; charset=utf-8")],
        state.runtime_graphql_schema().sdl(),
    )
        .into_response()
}
