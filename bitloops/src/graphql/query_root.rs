use super::context::DevqlGraphqlContext;
use super::types::Repository;
use async_graphql::{Context, Object};

#[derive(Default)]
pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn repo(&self, ctx: &Context<'_>, name: String) -> Repository {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .repository_for_name(&name)
    }
}
