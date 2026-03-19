use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguagePackContext {
    pub repo_root: PathBuf,
    pub repo_id: String,
    pub commit_sha: Option<String>,
}

impl LanguagePackContext {
    pub fn new(repo_root: PathBuf, repo_id: impl Into<String>, commit_sha: Option<String>) -> Self {
        Self {
            repo_root,
            repo_id: repo_id.into(),
            commit_sha,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityExecutionContext {
    pub repo_root: PathBuf,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub capability_pack_id: String,
    pub stage_id: String,
}

impl CapabilityExecutionContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_root: PathBuf,
        repo_id: impl Into<String>,
        commit_sha: Option<String>,
        capability_pack_id: impl Into<String>,
        stage_id: impl Into<String>,
    ) -> Self {
        Self {
            repo_root,
            repo_id: repo_id.into(),
            commit_sha,
            capability_pack_id: capability_pack_id.into(),
            stage_id: stage_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityIngestContext {
    pub repo_root: PathBuf,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub capability_pack_id: String,
    pub ingester_id: String,
}

impl CapabilityIngestContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_root: PathBuf,
        repo_id: impl Into<String>,
        commit_sha: Option<String>,
        capability_pack_id: impl Into<String>,
        ingester_id: impl Into<String>,
    ) -> Self {
        Self {
            repo_root,
            repo_id: repo_id.into(),
            commit_sha,
            capability_pack_id: capability_pack_id.into(),
            ingester_id: ingester_id.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_contexts_are_constructible_and_preserve_identity() {
        let repo_root = PathBuf::from("/tmp/repo");

        let language =
            LanguagePackContext::new(repo_root.clone(), "repo-id", Some("commit-sha".to_string()));
        assert_eq!(language.repo_id, "repo-id");
        assert_eq!(language.commit_sha.as_deref(), Some("commit-sha"));

        let execution = CapabilityExecutionContext::new(
            repo_root.clone(),
            "repo-id",
            Some("commit-sha".to_string()),
            "semantic-clones-pack",
            "semantic-clones",
        );
        assert_eq!(execution.capability_pack_id, "semantic-clones-pack");
        assert_eq!(execution.stage_id, "semantic-clones");

        let ingest = CapabilityIngestContext::new(
            repo_root,
            "repo-id",
            Some("commit-sha".to_string()),
            "knowledge-pack",
            "knowledge-ingester",
        );
        assert_eq!(ingest.capability_pack_id, "knowledge-pack");
        assert_eq!(ingest.ingester_id, "knowledge-ingester");
    }
}
