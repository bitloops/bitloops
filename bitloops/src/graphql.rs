mod context;
mod error;
mod loaders;
mod mutation_root;
mod query_root;
mod scope;
mod subscription_root;
mod types;

pub(crate) use context::DevqlGraphqlContext;
pub(crate) use error::{backend_error, bad_cursor_error, bad_user_input_error};
pub(crate) use scope::ResolverScope;

use self::loaders::LoaderRegistryExtension;
use self::mutation_root::MutationRoot;
use self::query_root::QueryRoot;
use self::subscription_root::SubscriptionRoot;
use async_graphql::Schema;
use async_graphql::http::{GraphQLPlaygroundConfig, playground_source};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::{
    extract::State,
    response::{Html, IntoResponse},
};

pub(crate) type DevqlSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;

pub(crate) fn build_schema(context: DevqlGraphqlContext) -> DevqlSchema {
    Schema::build(
        QueryRoot,
        async_graphql::EmptyMutation,
        async_graphql::EmptySubscription,
    )
    .data(context)
    .extension(LoaderRegistryExtension)
    .finish()
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
