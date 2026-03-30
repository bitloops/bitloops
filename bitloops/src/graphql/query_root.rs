use super::backend_error;
use super::context::DevqlGraphqlContext;
use super::types::{HealthStatus, Repository};
use async_graphql::{Context, Object, Result};

#[derive(Default)]
pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn health(&self, ctx: &Context<'_>) -> HealthStatus {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .health_status()
            .await
    }

    async fn repo(&self, ctx: &Context<'_>, name: String) -> Result<Repository> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .repository_for_name(&name)
            .await
            .map_err(|err| backend_error(format!("failed to resolve repository: {err:#}")))
    }
}
