use async_graphql::SimpleObject;

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct HealthBackendStatus {
    pub connected: bool,
    pub backend: String,
    pub status: String,
    pub detail: String,
}

impl HealthBackendStatus {
    pub fn new(
        connected: bool,
        backend: impl Into<String>,
        status: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            connected,
            backend: backend.into(),
            status: status.into(),
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct StorageAuthorityStatus {
    pub family: String,
    pub authority: String,
    pub backend: String,
}

impl StorageAuthorityStatus {
    pub fn new(
        family: impl Into<String>,
        authority: impl Into<String>,
        backend: impl Into<String>,
    ) -> Self {
        Self {
            family: family.into(),
            authority: authority.into(),
            backend: backend.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct HealthStatus {
    pub relational: HealthBackendStatus,
    pub events: HealthBackendStatus,
    pub blob: HealthBackendStatus,
    pub storage_authorities: Vec<StorageAuthorityStatus>,
}
