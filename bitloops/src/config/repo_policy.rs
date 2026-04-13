#[path = "repo_policy/discovery.rs"]
mod discovery;
#[path = "repo_policy/fingerprint.rs"]
mod fingerprint;
#[path = "repo_policy/merge.rs"]
mod merge;
#[path = "repo_policy/scope.rs"]
mod scope;
#[cfg(test)]
#[path = "repo_policy/tests.rs"]
mod tests;
#[path = "repo_policy/types.rs"]
mod types;

pub use self::discovery::{discover_repo_policy, discover_repo_policy_optional};
pub use self::scope::{parse_exclusion_patterns, resolve_repo_policy_scope_exclusions};
pub use self::types::{
    ImportedKnowledgeConfig, REPO_POLICY_FILE_NAME, REPO_POLICY_LOCAL_FILE_NAME,
    RepoPolicyExclusionFileReference, RepoPolicyScopeExclusions, RepoPolicySnapshot,
};
