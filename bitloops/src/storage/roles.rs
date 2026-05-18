use crate::config::{BlobStorageConfig, EventsBackendConfig, StoreBackendConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageRole {
    Runtime,
    CurrentProjection,
    SharedRelational,
    Events,
    RuntimeSessionBlobs,
    ProjectKnowledgeBlobs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobStorageRole {
    RuntimeSession,
    ProjectKnowledge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventStorageRole {
    CanonicalEvents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageAuthority {
    WorkspaceLocal,
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBackendKind {
    Sqlite,
    Postgres,
    DuckDb,
    ClickHouse,
    LocalDisk,
    S3,
    Gcs,
    Invalid,
}

impl StorageBackendKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
            Self::DuckDb => "duckdb",
            Self::ClickHouse => "clickhouse",
            Self::LocalDisk => "local",
            Self::S3 => "s3",
            Self::Gcs => "gcs",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageRoleResolver {
    shared_relational_backend: StorageBackendKind,
    canonical_events_backend: StorageBackendKind,
    project_blob_backend: StorageBackendKind,
}

impl StorageRoleResolver {
    pub fn from_backend_config(cfg: &StoreBackendConfig) -> Self {
        Self {
            shared_relational_backend: if cfg.relational.has_postgres() {
                StorageBackendKind::Postgres
            } else {
                StorageBackendKind::Sqlite
            },
            canonical_events_backend: resolve_events_backend(&cfg.events),
            project_blob_backend: resolve_project_blob_backend(&cfg.blobs),
        }
    }

    pub fn from_blob_config(cfg: &BlobStorageConfig) -> Self {
        Self {
            shared_relational_backend: StorageBackendKind::Sqlite,
            canonical_events_backend: StorageBackendKind::DuckDb,
            project_blob_backend: resolve_project_blob_backend(cfg),
        }
    }

    pub fn from_events_config(cfg: &EventsBackendConfig) -> Self {
        Self {
            shared_relational_backend: StorageBackendKind::Sqlite,
            canonical_events_backend: resolve_events_backend(cfg),
            project_blob_backend: StorageBackendKind::LocalDisk,
        }
    }

    pub const fn authority_for(self, role: StorageRole) -> StorageAuthority {
        match role {
            StorageRole::Runtime
            | StorageRole::CurrentProjection
            | StorageRole::RuntimeSessionBlobs => StorageAuthority::WorkspaceLocal,
            StorageRole::SharedRelational => match self.shared_relational_backend {
                StorageBackendKind::Postgres => StorageAuthority::Shared,
                _ => StorageAuthority::WorkspaceLocal,
            },
            StorageRole::Events => match self.canonical_events_backend {
                StorageBackendKind::ClickHouse => StorageAuthority::Shared,
                _ => StorageAuthority::WorkspaceLocal,
            },
            StorageRole::ProjectKnowledgeBlobs => match self.project_blob_backend {
                StorageBackendKind::S3 | StorageBackendKind::Gcs => StorageAuthority::Shared,
                _ => StorageAuthority::WorkspaceLocal,
            },
        }
    }

    pub const fn backend_for(self, role: StorageRole) -> StorageBackendKind {
        match role {
            StorageRole::Runtime | StorageRole::CurrentProjection => StorageBackendKind::Sqlite,
            StorageRole::SharedRelational => self.shared_relational_backend,
            StorageRole::Events => self.canonical_events_backend,
            StorageRole::RuntimeSessionBlobs => StorageBackendKind::LocalDisk,
            StorageRole::ProjectKnowledgeBlobs => self.project_blob_backend,
        }
    }

    pub const fn blob_backend_for(self, role: BlobStorageRole) -> StorageBackendKind {
        match role {
            BlobStorageRole::RuntimeSession => self.backend_for(StorageRole::RuntimeSessionBlobs),
            BlobStorageRole::ProjectKnowledge => {
                self.backend_for(StorageRole::ProjectKnowledgeBlobs)
            }
        }
    }

    pub const fn event_backend_for(self, role: EventStorageRole) -> StorageBackendKind {
        match role {
            EventStorageRole::CanonicalEvents => self.backend_for(StorageRole::Events),
        }
    }
}

fn resolve_events_backend(cfg: &EventsBackendConfig) -> StorageBackendKind {
    if cfg.has_clickhouse() {
        StorageBackendKind::ClickHouse
    } else {
        StorageBackendKind::DuckDb
    }
}

fn resolve_project_blob_backend(cfg: &BlobStorageConfig) -> StorageBackendKind {
    if cfg.s3_bucket.is_some() && cfg.gcs_bucket.is_some() {
        StorageBackendKind::Invalid
    } else if cfg.s3_bucket.is_some() {
        StorageBackendKind::S3
    } else if cfg.gcs_bucket.is_some() {
        StorageBackendKind::Gcs
    } else {
        StorageBackendKind::LocalDisk
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{
        BlobStorageConfig, EventsBackendConfig, RelationalBackendConfig, StoreBackendConfig,
    };

    use super::{
        BlobStorageRole, EventStorageRole, StorageAuthority, StorageBackendKind, StorageRole,
        StorageRoleResolver,
    };

    #[test]
    fn storage_role_resolver_reports_local_only_defaults() {
        let cfg = StoreBackendConfig {
            relational: RelationalBackendConfig {
                sqlite_path: None,
                postgres_dsn: None,
            },
            events: EventsBackendConfig {
                duckdb_path: None,
                clickhouse_url: None,
                clickhouse_user: None,
                clickhouse_password: None,
                clickhouse_database: None,
            },
            blobs: BlobStorageConfig {
                local_path: None,
                s3_bucket: None,
                s3_region: None,
                s3_access_key_id: None,
                s3_secret_access_key: None,
                gcs_bucket: None,
                gcs_credentials_path: None,
            },
        };

        let roles = StorageRoleResolver::from_backend_config(&cfg);

        assert_eq!(
            roles.authority_for(StorageRole::Runtime),
            StorageAuthority::WorkspaceLocal
        );
        assert_eq!(
            roles.backend_for(StorageRole::Runtime),
            StorageBackendKind::Sqlite
        );
        assert_eq!(
            roles.backend_for(StorageRole::CurrentProjection),
            StorageBackendKind::Sqlite
        );
        assert_eq!(
            roles.backend_for(StorageRole::SharedRelational),
            StorageBackendKind::Sqlite
        );
        assert_eq!(
            roles.backend_for(StorageRole::Events),
            StorageBackendKind::DuckDb
        );
        assert_eq!(
            roles.backend_for(StorageRole::RuntimeSessionBlobs),
            StorageBackendKind::LocalDisk
        );
        assert_eq!(
            roles.backend_for(StorageRole::ProjectKnowledgeBlobs),
            StorageBackendKind::LocalDisk
        );
        assert_eq!(
            roles.blob_backend_for(BlobStorageRole::RuntimeSession),
            StorageBackendKind::LocalDisk
        );
        assert_eq!(
            roles.blob_backend_for(BlobStorageRole::ProjectKnowledge),
            StorageBackendKind::LocalDisk
        );
        assert_eq!(
            roles.event_backend_for(EventStorageRole::CanonicalEvents),
            StorageBackendKind::DuckDb
        );
    }

    #[test]
    fn storage_role_resolver_reports_remote_authority_by_role() {
        let cfg = StoreBackendConfig {
            relational: RelationalBackendConfig {
                sqlite_path: None,
                postgres_dsn: Some("postgres://bitloops@postgres.internal/bitloops".to_string()),
            },
            events: EventsBackendConfig {
                duckdb_path: None,
                clickhouse_url: Some("http://clickhouse.internal:8123".to_string()),
                clickhouse_user: None,
                clickhouse_password: None,
                clickhouse_database: Some("analytics".to_string()),
            },
            blobs: BlobStorageConfig {
                local_path: None,
                s3_bucket: Some("bitloops-shared".to_string()),
                s3_region: Some("eu-central-1".to_string()),
                s3_access_key_id: None,
                s3_secret_access_key: None,
                gcs_bucket: None,
                gcs_credentials_path: None,
            },
        };

        let roles = StorageRoleResolver::from_backend_config(&cfg);

        assert_eq!(
            roles.authority_for(StorageRole::SharedRelational),
            StorageAuthority::Shared
        );
        assert_eq!(
            roles.backend_for(StorageRole::SharedRelational),
            StorageBackendKind::Postgres
        );
        assert_eq!(
            roles.authority_for(StorageRole::Events),
            StorageAuthority::Shared
        );
        assert_eq!(
            roles.event_backend_for(EventStorageRole::CanonicalEvents),
            StorageBackendKind::ClickHouse
        );
        assert_eq!(
            roles.backend_for(StorageRole::RuntimeSessionBlobs),
            StorageBackendKind::LocalDisk
        );
        assert_eq!(
            roles.blob_backend_for(BlobStorageRole::ProjectKnowledge),
            StorageBackendKind::S3
        );
    }
}
