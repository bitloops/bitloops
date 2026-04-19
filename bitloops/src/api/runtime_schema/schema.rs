use async_graphql::Schema;

use super::roots::{
    RuntimeMutationRoot, RuntimeQueryRoot, RuntimeRequestContext, RuntimeSubscriptionRoot,
};
use crate::api::DashboardState;
use crate::graphql::MAX_DEVQL_QUERY_DEPTH;

pub(crate) type RuntimeGraphqlSchema =
    Schema<RuntimeQueryRoot, RuntimeMutationRoot, RuntimeSubscriptionRoot>;

// The runtime surface is internal-only and powers init/status/dashboard operational views.
// Its snapshot query is intentionally richer than the public DevQL surfaces.
const MAX_RUNTIME_QUERY_COMPLEXITY: usize = 4096;

pub(crate) fn build_runtime_schema(
    state: DashboardState,
    request_context: RuntimeRequestContext,
) -> RuntimeGraphqlSchema {
    Schema::build(
        RuntimeQueryRoot,
        RuntimeMutationRoot,
        RuntimeSubscriptionRoot,
    )
    .data(state)
    .data(request_context)
    .limit_depth(MAX_DEVQL_QUERY_DEPTH)
    .limit_complexity(MAX_RUNTIME_QUERY_COMPLEXITY)
    .finish()
}

pub(crate) fn build_runtime_schema_template() -> RuntimeGraphqlSchema {
    Schema::build(
        RuntimeQueryRoot,
        RuntimeMutationRoot,
        RuntimeSubscriptionRoot,
    )
    .limit_depth(MAX_DEVQL_QUERY_DEPTH)
    .limit_complexity(MAX_RUNTIME_QUERY_COMPLEXITY)
    .finish()
}

pub fn runtime_schema_sdl() -> String {
    build_runtime_schema_template().sdl()
}
