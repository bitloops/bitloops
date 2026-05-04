use anyhow::{Result, bail};

use crate::models::{
    CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
    ProductionArtefact,
};

/// Host relational port for capability packs.
pub trait RelationalGateway: Send + Sync {
    fn resolve_checkpoint_id(&self, repo_id: &str, checkpoint_ref: &str) -> Result<String>;
    fn artefact_exists(&self, repo_id: &str, artefact_id: &str) -> Result<bool>;

    fn load_repo_id_for_commit(&self, commit_sha: &str) -> Result<String>;
    fn load_current_canonical_files(
        &self,
        repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalFileRecord>> {
        bail!(
            "current canonical file loading is not implemented by this relational gateway (repo {repo_id})"
        )
    }
    fn load_current_canonical_artefacts(
        &self,
        repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalArtefactRecord>> {
        bail!(
            "current canonical artefact loading is not implemented by this relational gateway (repo {repo_id})"
        )
    }
    fn load_current_canonical_edges(
        &self,
        repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalEdgeRecord>> {
        bail!(
            "current canonical edge loading is not implemented by this relational gateway (repo {repo_id})"
        )
    }
    fn load_current_production_artefacts(&self, repo_id: &str) -> Result<Vec<ProductionArtefact>>;
    fn load_production_artefacts(&self, commit_sha: &str) -> Result<Vec<ProductionArtefact>>;
    fn load_artefacts_for_file_lines(
        &self,
        commit_sha: &str,
        file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>>;
}
