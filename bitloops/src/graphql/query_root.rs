use super::context::DevqlGraphqlContext;
use super::types::{HealthStatus, Repository};
use async_graphql::{Context, Object};

#[derive(Default)]
pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn health(&self, ctx: &Context<'_>) -> HealthStatus {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .health_status()
            .await
    }

    async fn repo(&self, ctx: &Context<'_>, name: String) -> Repository {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .repository_for_name(&name)
    }
}
