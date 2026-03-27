mod context;
mod error;
mod loaders;
mod mutation_root;
mod pack_adapter;
mod query_root;
mod scope;
mod subscription_root;
mod subscriptions;
mod types;

pub(crate) use context::DevqlGraphqlContext;
pub(crate) use error::{backend_error, bad_cursor_error, bad_user_input_error};
pub(crate) use scope::{ResolvedTemporalScope, ResolverScope, TemporalAccessMode};
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
use self::subscription_root::SubscriptionRoot;
use anyhow::{Result, anyhow};
use async_graphql::http::{GraphQLPlaygroundConfig, playground_source};
use async_graphql::{Request, Schema, ServerError, Variables};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::{
    extract::State,
    response::{Html, IntoResponse},
};

pub(crate) type DevqlSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;

pub(crate) const MAX_DEVQL_QUERY_DEPTH: usize = 16;
pub(crate) const MAX_DEVQL_QUERY_COMPLEXITY: usize = 256;

pub(crate) fn build_schema(context: DevqlGraphqlContext) -> DevqlSchema {
    Schema::build(QueryRoot, MutationRoot, SubscriptionRoot)
        .data(context)
        .limit_depth(MAX_DEVQL_QUERY_DEPTH)
        .limit_complexity(MAX_DEVQL_QUERY_COMPLEXITY)
        .extension(LoaderRegistryExtension)
        .finish()
}

pub fn schema_sdl() -> String {
    build_schema(DevqlGraphqlContext::new(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        crate::api::DashboardDbPools::default(),
    ))
    .sdl()
}

pub(crate) async fn execute_in_process(
    repo_root: std::path::PathBuf,
    query: &str,
    variables: serde_json::Value,
) -> Result<serde_json::Value> {
    let schema = build_schema(DevqlGraphqlContext::new(
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

pub(crate) async fn graphql_handler(
    State(state): State<crate::api::DashboardState>,
    request: GraphQLRequest,
) -> GraphQLResponse {
    state
        .devql_schema()
        .execute(request.into_inner())
        .await
        .into()
}

pub(crate) async fn graphql_playground_handler() -> impl IntoResponse {
    Html(playground_source(
        GraphQLPlaygroundConfig::new("/devql")
            .subscription_endpoint("/devql/ws")
            .title("DevQL Explorer"),
    ))
}

pub(crate) async fn graphql_sdl_handler(
    State(state): State<crate::api::DashboardState>,
) -> impl IntoResponse {
    (
        [("content-type", "text/plain; charset=utf-8")],
        state.devql_schema().sdl(),
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
