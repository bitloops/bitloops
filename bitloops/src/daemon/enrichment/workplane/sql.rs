use std::path::Path;

use anyhow::{Context, Result};

use crate::host::devql::{RepoIdentity, esc_pg};

pub(crate) fn sql_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("converting enrichment runtime value to SQLite integer")
}

pub(crate) fn parse_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

pub(crate) fn sql_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

// Enrichment work items already carry the authoritative repo_id; workers must not
// rediscover identity from Git while processing hot-path batches.
pub(crate) fn repo_identity_from_runtime_metadata(repo_root: &Path, repo_id: &str) -> RepoIdentity {
    let name = repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repository")
        .to_string();
    RepoIdentity {
        provider: "runtime".to_string(),
        organization: "bound".to_string(),
        name: name.clone(),
        identity: format!("runtime://bound/{repo_id}/{name}"),
        repo_id: repo_id.to_string(),
    }
}
