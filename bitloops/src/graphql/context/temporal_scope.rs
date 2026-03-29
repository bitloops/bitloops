use anyhow::{Context, Result};
use serde_json::Value;
use tokio::task;

use super::DevqlGraphqlContext;
use crate::graphql::ResolverScope;
use crate::graphql::types::temporal_scope::{AsOfInput, AsOfSelector};
use crate::graphql::{ResolvedTemporalScope, TemporalAccessMode};
use crate::host::checkpoints::strategy::manual_commit::run_git;
use crate::host::devql::esc_pg;

impl DevqlGraphqlContext {
    pub(crate) async fn resolve_temporal_scope(
        &self,
        scope: &ResolverScope,
        input: &AsOfInput,
    ) -> Result<ResolvedTemporalScope> {
        match input.selector().map_err(anyhow::Error::msg)? {
            AsOfSelector::Ref(reference) => {
                let commit_sha = self.resolve_git_revision(scope, reference).await?;
                Ok(ResolvedTemporalScope::new(
                    commit_sha,
                    TemporalAccessMode::HistoricalCommit,
                ))
            }
            AsOfSelector::Commit(commit_sha) => {
                self.resolve_git_revision(scope, commit_sha).await?;
                Ok(ResolvedTemporalScope::new(
                    commit_sha.to_string(),
                    TemporalAccessMode::HistoricalCommit,
                ))
            }
            AsOfSelector::SaveCurrent => Ok(ResolvedTemporalScope::new(
                self.resolve_git_revision(scope, "HEAD").await?,
                TemporalAccessMode::SaveCurrent,
            )),
            AsOfSelector::SaveRevision(revision_id) => {
                let resolved_commit = self.resolve_save_revision_commit(scope, revision_id).await?;
                Ok(ResolvedTemporalScope::new(
                    resolved_commit,
                    TemporalAccessMode::SaveRevision(revision_id.to_string()),
                ))
            }
        }
    }

    async fn resolve_git_revision(&self, scope: &ResolverScope, revision: &str) -> Result<String> {
        let repo_root = self.repo_root_for_scope(scope)?;
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

    async fn resolve_save_revision_commit(
        &self,
        scope: &ResolverScope,
        revision_id: &str,
    ) -> Result<String> {
        let sqlite_path = self
            .backend_config
            .as_ref()
            .context("store backend configuration unavailable")?
            .relational
            .resolve_sqlite_db_path_for_repo(&self.config_root)
            .context("resolving SQLite path for GraphQL temporal scope")?;
        let branch = self.current_branch_name(scope);
        let repo_id = self.repo_id_for_scope(scope)?;
        let sql = format!(
            "SELECT commit_sha \
               FROM artefacts_current \
              WHERE repo_id = '{repo_id}' \
                AND branch = '{branch}' \
                AND revision_kind = 'temporary' \
                AND revision_id = '{revision_id}' \
              ORDER BY updated_at DESC \
              LIMIT 1",
            repo_id = esc_pg(&repo_id),
            branch = esc_pg(&branch),
            revision_id = esc_pg(revision_id),
        );
        let rows = self.query_sqlite_rows_at_path(&sqlite_path, &sql).await?;
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
