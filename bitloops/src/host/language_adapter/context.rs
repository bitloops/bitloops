use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanguageAdapterContext {
    pub(crate) repo_root: PathBuf,
    pub(crate) repo_id: String,
    pub(crate) commit_sha: Option<String>,
}

impl LanguageAdapterContext {
    pub(crate) fn new(
        repo_root: PathBuf,
        repo_id: impl Into<String>,
        commit_sha: Option<String>,
    ) -> Self {
        Self {
            repo_root,
            repo_id: repo_id.into(),
            commit_sha,
        }
    }
}
