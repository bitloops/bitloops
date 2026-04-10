mod context;
mod error;
mod loaders;
mod mutation_root;
mod pack_adapter;
mod query_root;
mod scope;
mod slim_query_root;
mod slim_subscription_root;
mod subscription_root;
mod subscriptions;
mod types;

pub(crate) use context::DevqlGraphqlContext;
pub(crate) use error::{backend_error, bad_cursor_error, bad_user_input_error};
pub(crate) use scope::{ResolvedTemporalScope, ResolverScope, TemporalAccessMode};
pub(crate) use subscriptions::SubscriptionHub;
pub(crate) use types::{ArtefactFilterInput, CanonicalKind};
pub(crate) use types::Checkpoint;

#[cfg(test)]
pub(crate) use types::DateTimeScalar;
#[cfg(test)]
pub(crate) use types::artefact::LineRangeInput;

use self::loaders::LoaderRegistryExtension;
use self::mutation_root::MutationRoot;
use self::query_root::QueryRoot;
use self::slim_query_root::SlimQueryRoot;
use self::slim_subscription_root::SlimSubscriptionRoot;
use self::subscription_root::SubscriptionRoot;
use anyhow::{Result, anyhow};
use async_graphql::http::{ALL_WEBSOCKET_PROTOCOLS, GraphQLPlaygroundConfig, playground_source};
use async_graphql::{Pos, Request, Response, Schema, ServerError, Variables};
use async_graphql_axum::{GraphQLProtocol, GraphQLRequest, GraphQLResponse, GraphQLWebSocket};
use axum::{
    extract::State,
    extract::WebSocketUpgrade,
    http::HeaderMap,
    response::{Html, IntoResponse, Response as AxumResponse},
};
use serde_json::{Value as JsonValue, json};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::devql_transport::{
    daemon_binding_identifier_for_config_path, parse_daemon_binding_header, parse_repo_root_header,
    parse_slim_cli_scope_headers, upsert_repo_path_registry_scope,
};

pub(crate) type DevqlSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;
pub(crate) type SlimDevqlSchema = Schema<SlimQueryRoot, MutationRoot, SlimSubscriptionRoot>;

pub(crate) const MAX_DEVQL_QUERY_DEPTH: usize = 16;
pub(crate) const MAX_DEVQL_QUERY_COMPLEXITY: usize = 256;

#[cfg(test)]
pub(crate) fn build_schema(context: DevqlGraphqlContext) -> DevqlSchema {
    build_global_schema(context)
}

pub(crate) fn build_global_schema(context: DevqlGraphqlContext) -> DevqlSchema {
    Schema::build(QueryRoot, MutationRoot, SubscriptionRoot)
        .data(context)
        .limit_depth(MAX_DEVQL_QUERY_DEPTH)
        .limit_complexity(MAX_DEVQL_QUERY_COMPLEXITY)
        .extension(LoaderRegistryExtension)
        .finish()
}

pub(crate) fn build_global_schema_template() -> DevqlSchema {
    Schema::build(QueryRoot, MutationRoot, SubscriptionRoot)
        .limit_depth(MAX_DEVQL_QUERY_DEPTH)
        .limit_complexity(MAX_DEVQL_QUERY_COMPLEXITY)
        .extension(LoaderRegistryExtension)
        .finish()
}

pub(crate) fn build_slim_schema(context: DevqlGraphqlContext) -> SlimDevqlSchema {
    Schema::build(SlimQueryRoot, MutationRoot, SlimSubscriptionRoot)
        .data(context)
        .limit_depth(MAX_DEVQL_QUERY_DEPTH)
        .limit_complexity(MAX_DEVQL_QUERY_COMPLEXITY)
        .extension(LoaderRegistryExtension)
        .finish()
}

pub(crate) fn build_slim_schema_template() -> SlimDevqlSchema {
    Schema::build(SlimQueryRoot, MutationRoot, SlimSubscriptionRoot)
        .limit_depth(MAX_DEVQL_QUERY_DEPTH)
        .limit_complexity(MAX_DEVQL_QUERY_COMPLEXITY)
        .extension(LoaderRegistryExtension)
        .finish()
}

pub fn schema_sdl() -> String {
    build_global_schema(DevqlGraphqlContext::new(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        crate::api::DashboardDbPools::default(),
    ))
    .sdl()
}

pub fn slim_schema_sdl() -> String {
    build_slim_schema(DevqlGraphqlContext::for_slim_request(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        Some("main".to_string()),
        None,
        None,
        true,
        crate::api::DashboardDbPools::default(),
    ))
    .sdl()
}

pub(crate) async fn execute_in_process(
    repo_root: std::path::PathBuf,
    query: &str,
    variables: serde_json::Value,
) -> Result<serde_json::Value> {
    let schema = build_global_schema(DevqlGraphqlContext::new(
        repo_root,
        crate::api::DashboardDbPools::default(),
    ));
    let response = schema
        .execute(Request::new(query).variables(Variables::from_json(variables)))
        .await;

    if let Some(error) = response.errors.first() {
        return Err(map_execution_error(error));
    }

    response
        .data
        .into_json()
        .map_err(|err| anyhow!("failed to decode GraphQL response payload: {err:#}"))
}

pub(crate) async fn slim_graphql_handler(
    State(state): State<crate::api::DashboardState>,
    headers: HeaderMap,
    request: GraphQLRequest,
) -> AxumResponse {
    let started = Instant::now();
    let request = request.into_inner();
    let signature = graphql_request_signature(&request);
    let scope = match parse_slim_cli_scope_headers(&headers) {
        Ok(scope) => scope,
        Err(err) => {
            let response = graphql_error_response(err).into_response();
            track_devql_action(DevqlGraphqlTelemetry {
                repo_root: state.repo_root.as_path(),
                event: "bitloops devql slim http",
                scope: "slim",
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
    if let Err(err) = validate_repo_daemon_binding(
        &headers,
        &state,
        scope.as_ref().map(|scope| scope.repo_root.as_path()),
    ) {
        let response = graphql_error_response(err).into_response();
        track_devql_action(DevqlGraphqlTelemetry {
            repo_root: state.repo_root.as_path(),
            event: "bitloops devql slim http",
            scope: "slim",
            transport: "http",
            request_kind: &signature.0,
            operation_family: &signature.1,
            success: false,
            status: response.status(),
            duration: started.elapsed(),
        });
        return response;
    }
    if let (Some(scope), Some(registry_path)) = (scope.as_ref(), state.repo_registry_path())
        && let Err(err) = upsert_repo_path_registry_scope(registry_path, scope)
    {
        let response = graphql_error_response(err).into_response();
        track_devql_action(DevqlGraphqlTelemetry {
            repo_root: state.repo_root.as_path(),
            event: "bitloops devql slim http",
            scope: "slim",
            transport: "http",
            request_kind: &signature.0,
            operation_family: &signature.1,
            success: false,
            status: response.status(),
            duration: started.elapsed(),
        });
        return response;
    }
    let repo_root = scope
        .as_ref()
        .map(|scope| scope.repo_root.clone())
        .unwrap_or_else(|| state.repo_root.clone());
    let context = DevqlGraphqlContext::for_slim_request(
        state.config_root.clone(),
        repo_root.clone(),
        scope.as_ref().map(|scope| scope.branch_name.clone()),
        scope.as_ref().and_then(|scope| scope.project_path.clone()),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        scope.is_some(),
        state.db.clone(),
    )
    .with_subscription_hub(state.subscription_hub());
    let (response, success) =
        execute_graphql_request(state.devql_slim_schema(), request.data(context), &headers).await;
    let response = response.into_response();
    track_devql_action(DevqlGraphqlTelemetry {
        repo_root: repo_root.as_path(),
        event: "bitloops devql slim http",
        scope: "slim",
        transport: "http",
        request_kind: &signature.0,
        operation_family: &signature.1,
        success,
        status: response.status(),
        duration: started.elapsed(),
    });
    response
}

pub(crate) async fn global_graphql_handler(
    State(state): State<crate::api::DashboardState>,
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
            track_devql_action(DevqlGraphqlTelemetry {
                repo_root: state.repo_root.as_path(),
                event: "bitloops devql global http",
                scope: "global",
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
        track_devql_action(DevqlGraphqlTelemetry {
            repo_root: repo_root.as_deref().unwrap_or(state.repo_root.as_path()),
            event: "bitloops devql global http",
            scope: "global",
            transport: "http",
            request_kind: &signature.0,
            operation_family: &signature.1,
            success: false,
            status: response.status(),
            duration: started.elapsed(),
        });
        return response;
    }
    let context = DevqlGraphqlContext::for_global_request(
        state.config_root.clone(),
        state.repo_root.clone(),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        state.db.clone(),
    )
    .with_subscription_hub(state.subscription_hub());
    let (response, success) =
        execute_graphql_request(state.devql_global_schema(), request.data(context), &headers).await;
    let response = response.into_response();
    track_devql_action(DevqlGraphqlTelemetry {
        repo_root: state.repo_root.as_path(),
        event: "bitloops devql global http",
        scope: "global",
        transport: "http",
        request_kind: &signature.0,
        operation_family: &signature.1,
        success,
        status: response.status(),
        duration: started.elapsed(),
    });
    response
}

pub(crate) async fn slim_graphql_ws_handler(
    State(state): State<crate::api::DashboardState>,
    protocol: GraphQLProtocol,
    upgrade: WebSocketUpgrade,
    headers: HeaderMap,
) -> impl IntoResponse {
    let started = Instant::now();
    let scope = match parse_slim_cli_scope_headers(&headers) {
        Ok(scope) => scope,
        Err(err) => {
            let response = graphql_error_response(err).into_response();
            track_devql_action(DevqlGraphqlTelemetry {
                repo_root: state.repo_root.as_path(),
                event: "bitloops devql slim ws",
                scope: "slim",
                transport: "ws",
                request_kind: "subscription",
                operation_family: "anonymous",
                success: false,
                status: response.status(),
                duration: started.elapsed(),
            });
            return response;
        }
    };
    if let Err(err) = validate_repo_daemon_binding(
        &headers,
        &state,
        scope.as_ref().map(|scope| scope.repo_root.as_path()),
    ) {
        let response = graphql_error_response(err).into_response();
        track_devql_action(DevqlGraphqlTelemetry {
            repo_root: state.repo_root.as_path(),
            event: "bitloops devql slim ws",
            scope: "slim",
            transport: "ws",
            request_kind: "subscription",
            operation_family: "anonymous",
            success: false,
            status: response.status(),
            duration: started.elapsed(),
        });
        return response;
    }
    if let (Some(scope), Some(registry_path)) = (scope.as_ref(), state.repo_registry_path())
        && let Err(err) = upsert_repo_path_registry_scope(registry_path, scope)
    {
        let response = graphql_error_response(err).into_response();
        track_devql_action(DevqlGraphqlTelemetry {
            repo_root: state.repo_root.as_path(),
            event: "bitloops devql slim ws",
            scope: "slim",
            transport: "ws",
            request_kind: "subscription",
            operation_family: "anonymous",
            success: false,
            status: response.status(),
            duration: started.elapsed(),
        });
        return response;
    }

    let repo_root = scope
        .as_ref()
        .map(|scope| scope.repo_root.clone())
        .unwrap_or_else(|| state.repo_root.clone());
    let context = DevqlGraphqlContext::for_slim_request(
        state.config_root.clone(),
        repo_root.clone(),
        scope.as_ref().map(|scope| scope.branch_name.clone()),
        scope.as_ref().and_then(|scope| scope.project_path.clone()),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        scope.is_some(),
        state.db.clone(),
    )
    .with_subscription_hub(state.subscription_hub());
    let schema = build_slim_schema(context);
    let response = upgrade
        .protocols(ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| GraphQLWebSocket::new(stream, schema, protocol).serve())
        .into_response();
    track_devql_action(DevqlGraphqlTelemetry {
        repo_root: repo_root.as_path(),
        event: "bitloops devql slim ws",
        scope: "slim",
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

pub(crate) async fn global_graphql_ws_handler(
    State(state): State<crate::api::DashboardState>,
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
    let context = DevqlGraphqlContext::for_global_request(
        state.config_root.clone(),
        state.repo_root.clone(),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        state.db.clone(),
    )
    .with_subscription_hub(state.subscription_hub());
    let schema = build_global_schema(context);

    let response = upgrade
        .protocols(ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| GraphQLWebSocket::new(stream, schema, protocol).serve())
        .into_response();
    track_devql_action(DevqlGraphqlTelemetry {
        repo_root: state.repo_root.as_path(),
        event: "bitloops devql global ws",
        scope: "global",
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

fn validate_repo_daemon_binding(
    headers: &HeaderMap,
    state: &crate::api::DashboardState,
    repo_root: Option<&Path>,
) -> Result<()> {
    let binding = parse_daemon_binding_header(headers)?;
    let Some(repo_root) = repo_root else {
        if binding.is_some() {
            anyhow::bail!(
                "This repo is not configured to work with the current Bitloops daemon. Run `bitloops init` to bind or rebind this repo."
            );
        }
        return Ok(());
    };

    let Some(binding) = binding else {
        anyhow::bail!(
            "This repo is not configured to work with the current Bitloops daemon. Run `bitloops init` to bind or rebind this repo."
        );
    };

    let expected = daemon_binding_identifier_for_config_path(
        &state
            .config_path
            .canonicalize()
            .unwrap_or_else(|_| state.config_path.clone()),
    );
    if binding == expected {
        return Ok(());
    }

    anyhow::bail!(
        "This repo at {} is not configured to work with the current Bitloops daemon. Run `bitloops init` to bind or rebind this repo.",
        repo_root.display()
    )
}

async fn execute_graphql_request<Query, Mutation, Subscription>(
    schema: &Schema<Query, Mutation, Subscription>,
    request: Request,
    headers: &HeaderMap,
) -> (GraphQLResponse, bool)
where
    Query: async_graphql::ObjectType + Send + Sync + 'static,
    Mutation: async_graphql::ObjectType + Send + Sync + 'static,
    Subscription: async_graphql::SubscriptionType + Send + Sync + 'static,
{
    if !crate::devql_timing::timings_requested(headers) {
        let response = schema.execute(request).await;
        let success = response.errors.is_empty();
        return (response.into(), success);
    }

    let query_bytes = request.query.len();
    let operation_name = request.operation_name.clone();
    let trace = crate::devql_timing::TimingTrace::new();
    let execute_started = Instant::now();
    let response = crate::devql_timing::scope_trace(trace.clone(), schema.execute(request)).await;
    let error_count = response.errors.len();
    let success = error_count == 0;
    trace.record(
        "server.graphql.execute",
        execute_started.elapsed(),
        json!({
            "queryBytes": query_bytes,
            "operationName": operation_name,
            "errors": error_count,
        }),
    );

    let timing_value = async_graphql::Value::from_json(trace.summary_value())
        .unwrap_or(async_graphql::Value::Null);

    (
        response
            .extension(crate::devql_timing::DEVQL_TIMINGS_EXTENSION, timing_value)
            .into(),
        success,
    )
}

fn graphql_request_signature(request: &Request) -> (String, String) {
    let request_kind = graphql_request_kind(request.query.as_str()).to_string();
    let raw_operation_name = request
        .operation_name
        .clone()
        .or_else(|| extract_operation_name(request.query.as_str()).map(str::to_string));
    let operation_name = raw_operation_name
        .as_deref()
        .and_then(graphql_operation_family)
        .unwrap_or_else(|| {
            if raw_operation_name.is_some() {
                "custom".to_string()
            } else {
                "anonymous".to_string()
            }
        });
    (request_kind, operation_name)
}

fn graphql_request_kind(query: &str) -> &'static str {
    let trimmed = query.trim_start();
    if trimmed.starts_with("mutation") {
        "mutation"
    } else if trimmed.starts_with("subscription") {
        "subscription"
    } else if trimmed.starts_with("query") || trimmed.starts_with('{') {
        "query"
    } else {
        "unknown"
    }
}

fn extract_operation_name(query: &str) -> Option<&str> {
    let trimmed = query.trim_start();
    let prefix = ["query", "mutation", "subscription"]
        .into_iter()
        .find(|prefix| trimmed.starts_with(prefix))?;
    let remainder = trimmed.strip_prefix(prefix)?.trim_start();
    let name_end = remainder
        .find(|ch: char| ch == '(' || ch == '{' || ch.is_whitespace())
        .unwrap_or(remainder.len());
    let name = remainder[..name_end].trim();
    (!name.is_empty()).then_some(name)
}

fn graphql_operation_family(name: &str) -> Option<String> {
    match name {
        "InitSchema"
        | "EnqueueTask"
        | "Task"
        | "Tasks"
        | "TaskQueue"
        | "PauseTaskQueue"
        | "ResumeTaskQueue"
        | "CancelTask"
        | "TaskProgress" => Some(name.to_string()),
        _ => None,
    }
}

/// Inputs for [`track_devql_action`]: DevQL GraphQL/WS request telemetry (HTTP or WebSocket).
struct DevqlGraphqlTelemetry<'a> {
    repo_root: &'a Path,
    event: &'a str,
    scope: &'a str,
    transport: &'a str,
    request_kind: &'a str,
    operation_family: &'a str,
    success: bool,
    status: axum::http::StatusCode,
    duration: Duration,
}

fn track_devql_action(t: DevqlGraphqlTelemetry<'_>) {
    let mut properties = HashMap::new();
    properties.insert("scope".to_string(), JsonValue::String(t.scope.to_string()));
    properties.insert(
        "transport".to_string(),
        JsonValue::String(t.transport.to_string()),
    );
    properties.insert(
        "request_kind".to_string(),
        JsonValue::String(t.request_kind.to_string()),
    );
    properties.insert(
        "operation_family".to_string(),
        JsonValue::String(t.operation_family.to_string()),
    );
    properties.insert(
        "status_code_class".to_string(),
        JsonValue::String(crate::api::status_code_class(t.status).to_string()),
    );

    crate::api::track_repo_action(
        t.repo_root,
        crate::telemetry::analytics::ActionDescriptor {
            event: t.event.to_string(),
            surface: if t.transport == "ws" {
                "devql_ws"
            } else {
                "devql_http"
            },
            properties,
        },
        t.success,
        t.duration,
    );
}

pub(crate) async fn slim_graphql_playground_handler() -> impl IntoResponse {
    graphql_playground_response("/devql", "/devql/ws", "DevQL Slim Explorer")
}

pub(crate) async fn global_graphql_playground_handler() -> impl IntoResponse {
    graphql_playground_response("/devql/global", "/devql/global/ws", "DevQL Global Explorer")
}

fn graphql_playground_response(
    endpoint: &str,
    subscription_endpoint: &str,
    title: &str,
) -> impl IntoResponse {
    Html(playground_source(
        GraphQLPlaygroundConfig::new(endpoint)
            .subscription_endpoint(subscription_endpoint)
            .title(title),
    ))
}

pub(crate) async fn slim_graphql_sdl_handler(
    State(state): State<crate::api::DashboardState>,
    headers: HeaderMap,
) -> AxumResponse {
    let scope = match parse_slim_cli_scope_headers(&headers) {
        Ok(scope) => scope,
        Err(err) => {
            return (axum::http::StatusCode::BAD_REQUEST, err.to_string()).into_response();
        }
    };
    if let Err(err) = validate_repo_daemon_binding(
        &headers,
        &state,
        scope.as_ref().map(|scope| scope.repo_root.as_path()),
    ) {
        return (axum::http::StatusCode::CONFLICT, err.to_string()).into_response();
    }
    (
        [("content-type", "text/plain; charset=utf-8")],
        state.devql_slim_schema().sdl(),
    )
        .into_response()
}

pub(crate) async fn global_graphql_sdl_handler(
    State(state): State<crate::api::DashboardState>,
    headers: HeaderMap,
) -> AxumResponse {
    let repo_root = match parse_repo_root_header(&headers) {
        Ok(repo_root) => repo_root,
        Err(err) => {
            return (axum::http::StatusCode::BAD_REQUEST, err.to_string()).into_response();
        }
    };
    if let Err(err) = validate_repo_daemon_binding(&headers, &state, repo_root.as_deref()) {
        return (axum::http::StatusCode::CONFLICT, err.to_string()).into_response();
    }
    (
        [("content-type", "text/plain; charset=utf-8")],
        state.devql_global_schema().sdl(),
    )
        .into_response()
}

fn map_execution_error(error: &ServerError) -> anyhow::Error {
    let code = error
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get("code"))
        .and_then(|value| match value {
            async_graphql::Value::String(value) => Some(value.as_str()),
            _ => None,
        });
    match code {
        Some("BAD_USER_INPUT") | Some("BAD_CURSOR") => anyhow!(error.message.clone()),
        Some(other) => anyhow!("{other}: {}", error.message),
        None => anyhow!("GraphQL execution failed: {}", error.message),
    }
}

fn graphql_error_response(err: anyhow::Error) -> GraphQLResponse {
    Response::from_errors(vec![
        bad_user_input_error(err.to_string()).into_server_error(Pos::default()),
    ])
    .into()
}

#[cfg(test)]
mod analytics_signature_tests {
    use super::*;

    #[test]
    fn graphql_request_signature_whitelists_known_operation_names() {
        let request = Request::new("mutation EnqueueTask($input: EnqueueTaskInput!) { enqueueTask(input: $input) { merged } }");
        let signature = graphql_request_signature(&request);

        assert_eq!(signature.0, "mutation");
        assert_eq!(signature.1, "EnqueueTask");
    }

    #[test]
    fn graphql_request_signature_marks_custom_operations_without_query_text_leakage() {
        let request = Request::new("query DashboardBranches { repositories { name } }");
        let signature = graphql_request_signature(&request);

        assert_eq!(signature.0, "query");
        assert_eq!(signature.1, "custom");
    }

    #[test]
    fn graphql_request_signature_marks_anonymous_requests() {
        let request = Request::new("{ repositories { name } }");
        let signature = graphql_request_signature(&request);

        assert_eq!(signature.0, "query");
        assert_eq!(signature.1, "anonymous");
    }
}

#[cfg(test)]
mod schema_template_tests {
    use super::{build_global_schema_template, build_slim_schema_template};

    #[test]
    fn global_and_slim_schema_templates_expose_non_empty_sdl() {
        let global_sdl = build_global_schema_template().sdl();
        assert!(global_sdl.len() > 500);
        assert!(global_sdl.contains("type Mutation"));

        let slim_sdl = build_slim_schema_template().sdl();
        assert!(slim_sdl.len() > 500);
        assert!(slim_sdl.contains("type Mutation"));
        assert_ne!(
            global_sdl, slim_sdl,
            "slim schema should differ from global SDL"
        );
    }
}
