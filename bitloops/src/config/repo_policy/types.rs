use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

pub const REPO_POLICY_FILE_NAME: &str = ".bitloops.toml";
pub const REPO_POLICY_LOCAL_FILE_NAME: &str = ".bitloops.local.toml";

#[derive(Debug, Clone)]
pub struct RepoPolicySnapshot {
    pub root: Option<PathBuf>,
    pub shared_path: Option<PathBuf>,
    pub local_path: Option<PathBuf>,
    pub daemon_config_path: Option<PathBuf>,
    pub capture: Value,
    pub watch: Value,
    pub devql: Value,
    pub scope: Value,
    pub contexts: Value,
    pub agents: Value,
    pub knowledge_import_paths: Vec<String>,
    pub imported_knowledge: Vec<ImportedKnowledgeConfig>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPolicyScopeExclusions {
    pub exclude: Vec<String>,
    pub exclude_from: Vec<String>,
    pub referenced_files: Vec<RepoPolicyExclusionFileReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPolicyExclusionFileReference {
    pub configured_path: String,
    pub resolved_path: PathBuf,
    pub content: String,
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ImportedKnowledgeConfig {
    pub path: PathBuf,
    pub value: Value,
}

pub(super) struct RepoPolicyFingerprintInputs<'a> {
    pub(super) capture: &'a Value,
    pub(super) watch: &'a Value,
    pub(super) devql: &'a Value,
    pub(super) scope: &'a Value,
    pub(super) scope_exclusions: &'a RepoPolicyScopeExclusions,
    pub(super) contexts: &'a Value,
    pub(super) agents: &'a Value,
    pub(super) knowledge_import_paths: &'a [String],
    pub(super) imported_knowledge: &'a [ImportedKnowledgeConfig],
}

#[derive(Debug, Clone)]
pub(super) struct RepoPolicyLocation {
    pub(super) root: PathBuf,
    pub(super) shared_path: Option<PathBuf>,
    pub(super) local_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct RepoPolicyTomlFile {
    #[serde(default)]
    pub(super) capture: Option<Value>,
    #[serde(default)]
    pub(super) watch: Option<Value>,
    #[serde(default)]
    pub(super) devql: Option<Value>,
    #[serde(default)]
    pub(super) scope: Option<Value>,
    #[serde(default)]
    pub(super) contexts: Option<Value>,
    #[serde(default)]
    pub(super) agents: Option<Value>,
    #[serde(default)]
    pub(super) daemon: RepoPolicyDaemon,
    #[serde(default)]
    pub(super) imports: RepoPolicyImports,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct RepoPolicyDaemon {
    #[serde(default)]
    pub(super) config_path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct RepoPolicyImports {
    #[serde(default)]
    pub(super) knowledge: Vec<String>,
}
