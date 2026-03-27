use anyhow::{Context, Result};
use serde_json::Value;
use tokio::task;

use super::DevqlGraphqlContext;
use super::git_history::git_default_branch_name;
use crate::graphql::types::temporal_scope::{AsOfInput, AsOfSelector};
use crate::graphql::{ResolvedTemporalScope, TemporalAccessMode};
use crate::host::checkpoints::strategy::manual_commit::run_git;
use crate::host::devql::{esc_pg, sqlite_query_rows_path};

impl DevqlGraphqlContext {
    pub(crate) async fn resolve_temporal_scope(
        &self,
        input: &AsOfInput,
    ) -> Result<ResolvedTemporalScope> {
        match input.selector().map_err(anyhow::Error::msg)? {
            AsOfSelector::Ref(reference) => {
                let commit_sha = self.resolve_git_revision(reference).await?;
                Ok(ResolvedTemporalScope::new(
                    commit_sha,
                    TemporalAccessMode::HistoricalCommit,
                ))
            }
            AsOfSelector::Commit(commit_sha) => {
                self.resolve_git_revision(commit_sha).await?;
                Ok(ResolvedTemporalScope::new(
                    commit_sha.to_string(),
                    TemporalAccessMode::HistoricalCommit,
                ))
            }
            AsOfSelector::SaveCurrent => Ok(ResolvedTemporalScope::new(
                self.resolve_git_revision("HEAD").await?,
                TemporalAccessMode::SaveCurrent,
            )),
            AsOfSelector::SaveRevision(revision_id) => {
                let resolved_commit = self.resolve_save_revision_commit(revision_id).await?;
                Ok(ResolvedTemporalScope::new(
                    resolved_commit,
                    TemporalAccessMode::SaveRevision(revision_id.to_string()),
                ))
            }
        }
    }

    async fn resolve_git_revision(&self, revision: &str) -> Result<String> {
        let repo_root = self.repo_root.clone();
        let revision = revision.trim().to_string();
        let revision_for_log = revision.clone();
        task::spawn_blocking(move || {
            run_git(repo_root.as_path(), &["rev-parse", revision.as_str()])
                .map(|value| value.trim().to_string())
        })
        .await
        .with_context(|| format!("joining git rev-parse task for `{revision_for_log}`"))?
        .with_context(|| format!("resolving git revision `{revision_for_log}`"))
    }

    async fn resolve_save_revision_commit(&self, revision_id: &str) -> Result<String> {
        let sqlite_path = self
            .backend_config
            .as_ref()
            .context("store backend configuration unavailable")?
            .relational
            .resolve_sqlite_db_path_for_repo(&self.repo_root)
            .context("resolving SQLite path for GraphQL temporal scope")?;
        let branch = git_default_branch_name(self.repo_root.as_path());
        let sql = format!(
            "SELECT commit_sha \
               FROM artefacts_current \
              WHERE repo_id = '{repo_id}' \
                AND branch = '{branch}' \
                AND revision_kind = 'temporary' \
                AND revision_id = '{revision_id}' \
              ORDER BY updated_at DESC \
              LIMIT 1",
            repo_id = esc_pg(self.repo_identity.repo_id.as_str()),
            branch = esc_pg(&branch),
            revision_id = esc_pg(revision_id),
        );
        let rows = sqlite_query_rows_path(&sqlite_path, &sql).await?;
        rows.into_iter()
            .find_map(|row| {
                row.get("commit_sha")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .with_context(|| format!("unknown save revision `{revision_id}`"))
    }
}
