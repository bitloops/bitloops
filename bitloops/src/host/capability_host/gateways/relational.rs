use anyhow::Result;

use crate::models::ProductionArtefact;

/// Host relational port for capability packs.
pub trait RelationalGateway: Send + Sync {
    fn resolve_checkpoint_id(&self, repo_id: &str, checkpoint_ref: &str) -> Result<String>;
    fn artefact_exists(&self, repo_id: &str, artefact_id: &str) -> Result<bool>;

    fn load_repo_id_for_commit(&self, commit_sha: &str) -> Result<String>;
    fn load_production_artefacts(&self, commit_sha: &str) -> Result<Vec<ProductionArtefact>>;
    fn load_artefacts_for_file_lines(
        &self,
        commit_sha: &str,
        file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>>;
}
