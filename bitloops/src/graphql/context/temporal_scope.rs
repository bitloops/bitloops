use anyhow::{Context, Result};
use tokio::task;

use super::DevqlGraphqlContext;
use crate::graphql::ResolverScope;
use crate::graphql::types::temporal_scope::{AsOfInput, AsOfSelector};
use crate::graphql::{ResolvedTemporalScope, TemporalAccessMode};
use crate::host::checkpoints::strategy::manual_commit::run_git;

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
                let resolved_commit = self
                    .resolve_save_revision_commit(scope, revision_id)
                    .await?;
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
        let _ = revision_id;
        // Sync-shaped artefacts_current no longer stores per-save-revision rows; anchor
        // save-revision scopes to Git HEAD for checkpoint / event-time correlation.
        self.resolve_git_revision(scope, "HEAD").await
    }
}
