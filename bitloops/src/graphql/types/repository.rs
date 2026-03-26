use async_graphql::{ID, SimpleObject};

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct Repository {
    pub id: ID,
    pub name: String,
    pub provider: String,
    pub organization: String,
}

impl Repository {
    pub fn new(name: &str, provider: &str, organization: &str) -> Self {
        Self {
            id: ID(format!("repo://{provider}/{organization}/{name}")),
            name: name.to_string(),
            provider: provider.to_string(),
            organization: organization.to_string(),
        }
    }
}
