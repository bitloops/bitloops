use std::path::{Path, PathBuf};

use async_graphql::{ID, InputObject, SimpleObject, types::Json};
use serde_json::Value;

use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, REPO_POLICY_FILE_NAME, REPO_POLICY_LOCAL_FILE_NAME,
};

type ConfigJsonScalar = Json<Value>;

pub(super) const REDACTED_VALUE: &str = "********";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConfigTargetKind {
    Daemon,
    RepoShared,
    RepoLocal,
}

impl ConfigTargetKind {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Daemon => "daemon",
            Self::RepoShared => "repo_shared",
            Self::RepoLocal => "repo_local",
        }
    }

    pub(super) fn scope_label(&self) -> &'static str {
        match self {
            Self::Daemon => "Daemon",
            Self::RepoShared => "Shared repo",
            Self::RepoLocal => "Local repo",
        }
    }

    pub(super) fn from_path(path: &Path) -> Option<Self> {
        match path.file_name().and_then(|name| name.to_str()) {
            Some(BITLOOPS_CONFIG_RELATIVE_PATH) => Some(Self::Daemon),
            Some(REPO_POLICY_FILE_NAME) => Some(Self::RepoShared),
            Some(REPO_POLICY_LOCAL_FILE_NAME) => Some(Self::RepoLocal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ConfigTarget {
    pub(super) id: ID,
    pub(super) kind: ConfigTargetKind,
    pub(super) label: String,
    pub(super) group: String,
    pub(super) path: PathBuf,
    pub(super) repo_root: Option<PathBuf>,
    pub(super) exists: bool,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeConfigTargetObject {
    pub(crate) id: ID,
    pub(crate) kind: String,
    pub(crate) scope: String,
    pub(crate) label: String,
    pub(crate) group: String,
    pub(crate) path: String,
    pub(crate) repo_root: Option<String>,
    pub(crate) exists: bool,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeConfigFieldObject {
    pub(crate) key: String,
    pub(crate) path: Vec<String>,
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) field_type: String,
    pub(crate) value: ConfigJsonScalar,
    pub(crate) effective_value: Option<ConfigJsonScalar>,
    pub(crate) default_value: Option<ConfigJsonScalar>,
    pub(crate) allowed_values: Vec<String>,
    pub(crate) validation_hints: Vec<String>,
    pub(crate) required: bool,
    pub(crate) read_only: bool,
    pub(crate) secret: bool,
    pub(crate) order: i32,
    pub(crate) source: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeConfigSectionObject {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) order: i32,
    pub(crate) advanced: bool,
    pub(crate) fields: Vec<RuntimeConfigFieldObject>,
    pub(crate) value: ConfigJsonScalar,
    pub(crate) effective_value: Option<ConfigJsonScalar>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeConfigSnapshotObject {
    pub(crate) target: RuntimeConfigTargetObject,
    pub(crate) revision: String,
    pub(crate) valid: bool,
    pub(crate) validation_errors: Vec<String>,
    pub(crate) restart_required: bool,
    pub(crate) reload_required: bool,
    pub(crate) sections: Vec<RuntimeConfigSectionObject>,
    pub(crate) raw_value: ConfigJsonScalar,
    pub(crate) effective_value: Option<ConfigJsonScalar>,
}

#[derive(Debug, Clone, InputObject)]
pub(crate) struct RuntimeConfigFieldPatchInput {
    pub(crate) path: Vec<String>,
    #[graphql(default)]
    pub(crate) value: Option<ConfigJsonScalar>,
    #[graphql(default)]
    pub(crate) unset: Option<bool>,
}

#[derive(Debug, Clone, InputObject)]
pub(crate) struct UpdateRuntimeConfigInput {
    pub(crate) target_id: ID,
    pub(crate) expected_revision: String,
    pub(crate) patches: Vec<RuntimeConfigFieldPatchInput>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct UpdateRuntimeConfigResult {
    pub(crate) snapshot: RuntimeConfigSnapshotObject,
    pub(crate) restart_required: bool,
    pub(crate) reload_required: bool,
    pub(crate) path: String,
    pub(crate) message: String,
}

impl From<ConfigTarget> for RuntimeConfigTargetObject {
    fn from(target: ConfigTarget) -> Self {
        Self {
            id: target.id,
            kind: target.kind.as_str().to_string(),
            scope: target.kind.scope_label().to_string(),
            label: target.label,
            group: target.group,
            path: target.path.display().to_string(),
            repo_root: target.repo_root.map(|path| path.display().to_string()),
            exists: target.exists,
        }
    }
}
