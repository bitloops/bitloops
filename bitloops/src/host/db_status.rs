use std::path::Path;

use crate::config::{BlobStorageConfig, StoreBackendConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseConnectionStatus {
    Connected,
    CouldNotAuthenticate,
    CouldNotReachDb,
    NotConfigured,
    Error,
}

impl DatabaseConnectionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::CouldNotAuthenticate => "Could not authenticate",
            Self::CouldNotReachDb => "Could not reach DB",
            Self::NotConfigured => "Not configured",
            Self::Error => "Error",
        }
    }

    pub fn is_failure(self) -> bool {
        matches!(
            self,
            Self::CouldNotAuthenticate | Self::CouldNotReachDb | Self::Error
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseStatusRow {
    pub db: &'static str,
    pub status: DatabaseConnectionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageAuthorityRow {
    pub family: &'static str,
    pub authority: &'static str,
    pub backend: &'static str,
}

pub fn classify_connection_error(message: &str) -> DatabaseConnectionStatus {
    let lowered = message.to_ascii_lowercase();

    if contains_any(
        &lowered,
        &[
            "password authentication failed",
            "authentication failed",
            "could not authenticate",
            "invalid password",
            "invalid username",
            "unauthorized",
            "forbidden",
            "401",
            "403",
            "access denied",
            "no pg_hba.conf entry",
        ],
    ) {
        return DatabaseConnectionStatus::CouldNotAuthenticate;
    }

    if contains_any(
        &lowered,
        &[
            "could not connect",
            "connection refused",
            "failed to connect",
            "couldn't connect",
            "timed out",
            "timeout",
            "no route to host",
            "name or service not known",
            "temporary failure in name resolution",
            "network is unreachable",
            "operation not permitted",
            "connection to server at",
            "failed to lookup address information",
        ],
    ) {
        return DatabaseConnectionStatus::CouldNotReachDb;
    }

    DatabaseConnectionStatus::Error
}

fn contains_any(input: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| input.contains(marker))
}

pub fn collect_storage_authority_rows(
    _config_root: &Path,
    _repo_root: &Path,
    cfg: &StoreBackendConfig,
) -> Vec<StorageAuthorityRow> {
    vec![
        StorageAuthorityRow {
            family: "runtime",
            authority: "workspace-local",
            backend: "sqlite",
        },
        StorageAuthorityRow {
            family: "relational current",
            authority: "workspace-local",
            backend: "sqlite",
        },
        StorageAuthorityRow {
            family: "relational shared",
            authority: if cfg.relational.has_postgres() {
                "shared"
            } else {
                "workspace-local"
            },
            backend: if cfg.relational.has_postgres() {
                "postgres"
            } else {
                "sqlite"
            },
        },
        StorageAuthorityRow {
            family: "events",
            authority: if cfg.events.has_clickhouse() {
                "shared"
            } else {
                "workspace-local"
            },
            backend: if cfg.events.has_clickhouse() {
                "clickhouse"
            } else {
                "duckdb"
            },
        },
        StorageAuthorityRow {
            family: "blob runtime/session",
            authority: "workspace-local",
            backend: "local",
        },
        StorageAuthorityRow {
            family: "blob project/knowledge",
            authority: project_blob_authority(&cfg.blobs),
            backend: project_blob_backend(&cfg.blobs),
        },
    ]
}

fn project_blob_authority(cfg: &BlobStorageConfig) -> &'static str {
    if cfg.s3_bucket.is_some() || cfg.gcs_bucket.is_some() {
        "shared"
    } else {
        "workspace-local"
    }
}

fn project_blob_backend(cfg: &BlobStorageConfig) -> &'static str {
    if cfg.s3_bucket.is_some() && cfg.gcs_bucket.is_some() {
        "invalid"
    } else if cfg.s3_bucket.is_some() {
        "s3"
    } else if cfg.gcs_bucket.is_some() {
        "gcs"
    } else {
        "local"
    }
}

#[cfg(test)]
mod tests {
    use super::collect_storage_authority_rows;
    use crate::config::{
        BlobStorageConfig, EventsBackendConfig, RelationalBackendConfig, StoreBackendConfig,
    };
    use tempfile::tempdir;

    #[test]
    fn storage_authority_rows_default_to_workspace_local_backends() {
        let temp = tempdir().expect("temp dir");
        let config_root = temp.path().join("daemon");
        let repo_root = temp.path().join("repo");
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

        let rows = collect_storage_authority_rows(&config_root, &repo_root, &cfg);

        assert_row(&rows, "runtime", "workspace-local", "sqlite");
        assert_row(&rows, "relational current", "workspace-local", "sqlite");
        assert_row(&rows, "relational shared", "workspace-local", "sqlite");
        assert_row(&rows, "events", "workspace-local", "duckdb");
        assert_row(&rows, "blob runtime/session", "workspace-local", "local");
        assert_row(&rows, "blob project/knowledge", "workspace-local", "local");
    }

    #[test]
    fn storage_authority_rows_mark_remote_shared_backends_when_configured() {
        let temp = tempdir().expect("temp dir");
        let config_root = temp.path().join("daemon");
        let repo_root = temp.path().join("repo");
        let cfg = StoreBackendConfig {
            relational: RelationalBackendConfig {
                sqlite_path: None,
                postgres_dsn: Some(
                    "postgres://bitloops:secret@postgres.internal:5432/bitloops".to_string(),
                ),
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

        let rows = collect_storage_authority_rows(&config_root, &repo_root, &cfg);

        assert_row(&rows, "runtime", "workspace-local", "sqlite");
        assert_row(&rows, "relational current", "workspace-local", "sqlite");
        assert_row(&rows, "relational shared", "shared", "postgres");
        assert_row(&rows, "events", "shared", "clickhouse");
        assert_row(&rows, "blob runtime/session", "workspace-local", "local");
        assert_row(&rows, "blob project/knowledge", "shared", "s3");
    }

    fn assert_row(
        rows: &[super::StorageAuthorityRow],
        family: &str,
        authority: &str,
        backend: &str,
    ) {
        let row = rows
            .iter()
            .find(|row| row.family == family)
            .unwrap_or_else(|| panic!("missing storage authority row for family `{family}`"));
        assert_eq!(row.authority, authority, "authority for `{family}`");
        assert_eq!(row.backend, backend, "backend for `{family}`");
    }
}
