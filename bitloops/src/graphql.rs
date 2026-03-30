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

#[cfg(test)]
pub(crate) use types::DateTimeScalar;
#[cfg(test)]
pub(crate) use types::artefact::LineRangeInput;
#[cfg(test)]
pub(crate) use types::ingestion::IngestionPhase;
#[cfg(test)]
pub(crate) use types::{Checkpoint, IngestionProgressEvent};

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
    response::{Html, IntoResponse},
};
use serde_json::json;
use std::time::Instant;

use crate::devql_transport::{parse_slim_cli_scope_headers, upsert_repo_path_registry_scope};

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
) -> GraphQLResponse {
    let request = request.into_inner();
    let scope = match parse_slim_cli_scope_headers(&headers) {
        Ok(scope) => scope,
        Err(err) => return graphql_error_response(err),
    };
    if let (Some(scope), Some(registry_path)) = (scope.as_ref(), state.repo_registry_path())
        && let Err(err) = upsert_repo_path_registry_scope(registry_path, scope)
    {
        return graphql_error_response(err);
    }
    let repo_root = scope
        .as_ref()
        .map(|scope| scope.repo_root.clone())
        .unwrap_or_else(|| state.repo_root.clone());
    let context = DevqlGraphqlContext::for_slim_request(
        state.config_root.clone(),
        repo_root,
        scope.as_ref().map(|scope| scope.branch_name.clone()),
        scope.as_ref().and_then(|scope| scope.project_path.clone()),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        scope.is_some(),
        state.db.clone(),
    )
    .with_subscription_hub(state.subscription_hub());
    execute_graphql_request(state.devql_slim_schema(), request.data(context), &headers).await
}

pub(crate) async fn global_graphql_handler(
    State(state): State<crate::api::DashboardState>,
    headers: HeaderMap,
    request: GraphQLRequest,
) -> GraphQLResponse {
    let request = request.into_inner();
    let context = DevqlGraphqlContext::for_global_request(
        state.config_root.clone(),
        state.repo_root.clone(),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        state.db.clone(),
    )
    .with_subscription_hub(state.subscription_hub());
    execute_graphql_request(state.devql_global_schema(), request.data(context), &headers).await
}

pub(crate) async fn slim_graphql_ws_handler(
    State(state): State<crate::api::DashboardState>,
    protocol: GraphQLProtocol,
    upgrade: WebSocketUpgrade,
    headers: HeaderMap,
) -> impl IntoResponse {
    let scope = match parse_slim_cli_scope_headers(&headers) {
        Ok(scope) => scope,
        Err(err) => return graphql_error_response(err).into_response(),
    };
    if let (Some(scope), Some(registry_path)) = (scope.as_ref(), state.repo_registry_path())
        && let Err(err) = upsert_repo_path_registry_scope(registry_path, scope)
    {
        return graphql_error_response(err).into_response();
    }

    let repo_root = scope
        .as_ref()
        .map(|scope| scope.repo_root.clone())
        .unwrap_or_else(|| state.repo_root.clone());
    let context = DevqlGraphqlContext::for_slim_request(
        state.config_root.clone(),
        repo_root,
        scope.as_ref().map(|scope| scope.branch_name.clone()),
        scope.as_ref().and_then(|scope| scope.project_path.clone()),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        scope.is_some(),
        state.db.clone(),
    )
    .with_subscription_hub(state.subscription_hub());
    let schema = build_slim_schema(context);

    upgrade
        .protocols(ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| GraphQLWebSocket::new(stream, schema, protocol).serve())
        .into_response()
}

pub(crate) async fn global_graphql_ws_handler(
    State(state): State<crate::api::DashboardState>,
    protocol: GraphQLProtocol,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    let context = DevqlGraphqlContext::for_global_request(
        state.config_root.clone(),
        state.repo_root.clone(),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        state.db.clone(),
    )
    .with_subscription_hub(state.subscription_hub());
    let schema = build_global_schema(context);

    upgrade
        .protocols(ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| GraphQLWebSocket::new(stream, schema, protocol).serve())
        .into_response()
}

async fn execute_graphql_request<Query, Mutation, Subscription>(
    schema: &Schema<Query, Mutation, Subscription>,
    request: Request,
    headers: &HeaderMap,
) -> GraphQLResponse
where
    Query: async_graphql::ObjectType + Send + Sync + 'static,
    Mutation: async_graphql::ObjectType + Send + Sync + 'static,
    Subscription: async_graphql::SubscriptionType + Send + Sync + 'static,
{
    if !crate::devql_timing::timings_requested(headers) {
        return schema.execute(request).await.into();
    }

    let query_bytes = request.query.len();
    let operation_name = request.operation_name.clone();
    let trace = crate::devql_timing::TimingTrace::new();
    let execute_started = Instant::now();
    let response = crate::devql_timing::scope_trace(trace.clone(), schema.execute(request)).await;
    let error_count = response.errors.len();
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

    response
        .extension(crate::devql_timing::DEVQL_TIMINGS_EXTENSION, timing_value)
        .into()
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
) -> impl IntoResponse {
    (
        [("content-type", "text/plain; charset=utf-8")],
        state.devql_slim_schema().sdl(),
    )
}

pub(crate) async fn global_graphql_sdl_handler(
    State(state): State<crate::api::DashboardState>,
) -> impl IntoResponse {
    (
        [("content-type", "text/plain; charset=utf-8")],
        state.devql_global_schema().sdl(),
    )
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
