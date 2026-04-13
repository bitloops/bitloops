use std::collections::BTreeMap;

use crate::host::devql::normalize_repo_path;

use super::path_rules::{
    file_name, is_jsconfig_file, is_lockfile_name, is_tsconfig_file, lower_extension,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnalysisMode {
    Code,
    Text,
    TrackOnly,
    Excluded,
}

impl AnalysisMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Text => "text",
            Self::TrackOnly => "track_only",
            Self::Excluded => "excluded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileRole {
    SourceCode,
    ProjectManifest,
    ContextSeed,
    Configuration,
    Documentation,
    Lockfile,
    Generated,
    DependencyTree,
    Other,
}

impl FileRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SourceCode => "source_code",
            Self::ProjectManifest => "project_manifest",
            Self::ContextSeed => "context_seed",
            Self::Configuration => "configuration",
            Self::Documentation => "documentation",
            Self::Lockfile => "lockfile",
            Self::Generated => "generated",
            Self::DependencyTree => "dependency_tree",
            Self::Other => "other",
        }
    }

    pub(crate) fn for_auto_ignored_path(path: &str) -> Self {
        let path = normalize_repo_path(path);
        if path.contains("/node_modules/")
            || path.starts_with("node_modules/")
            || path.contains("/vendor/")
            || path.starts_with("vendor/")
        {
            Self::DependencyTree
        } else {
            Self::Generated
        }
    }

    pub(crate) fn for_text_path(path: &str) -> Self {
        let file_name = file_name(path);
        if file_name.starts_with("README")
            || file_name.starts_with("CHANGELOG")
            || file_name.starts_with("LICENSE")
            || matches!(
                lower_extension(path).as_deref(),
                Some("md" | "mdx" | "txt" | "rst" | "adoc")
            )
        {
            return Self::Documentation;
        }
        if matches!(
            file_name,
            "Cargo.toml"
                | "package.json"
                | "pyproject.toml"
                | "setup.py"
                | "setup.cfg"
                | "Pipfile"
                | "go.mod"
                | "pom.xml"
        ) {
            return Self::ProjectManifest;
        }
        if is_tsconfig_file(file_name)
            || is_jsconfig_file(file_name)
            || file_name.starts_with("build.gradle")
        {
            return Self::ContextSeed;
        }
        Self::Configuration
    }

    pub(crate) fn for_track_only_path(path: &str, reason: &str) -> Self {
        let file_name = file_name(path);
        if is_lockfile_name(file_name) {
            return Self::Lockfile;
        }
        match reason {
            "auto_ignored" => Self::for_auto_ignored_path(path),
            "contextless_code_like" => Self::SourceCode,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextIndexMode {
    Embed,
    StoreOnly,
    None,
}

impl TextIndexMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Embed => "embed",
            Self::StoreOnly => "store_only",
            Self::None => "none",
        }
    }

    pub(crate) fn for_role(role: FileRole) -> Self {
        match role {
            FileRole::Documentation => Self::Embed,
            FileRole::ProjectManifest | FileRole::ContextSeed | FileRole::Configuration => {
                Self::StoreOnly
            }
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectContext {
    pub(crate) context_id: String,
    pub(crate) root: String,
    pub(crate) kind: String,
    pub(crate) detection_source: String,
    pub(crate) config_files: Vec<String>,
    pub(crate) config_fingerprint: String,
    pub(crate) base_languages: Vec<String>,
    pub(crate) frameworks: Vec<String>,
    pub(crate) runtime_profile: Option<String>,
    pub(crate) source_versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedFileClassification {
    pub(crate) analysis_mode: AnalysisMode,
    pub(crate) file_role: FileRole,
    pub(crate) text_index_mode: TextIndexMode,
    pub(crate) language: String,
    pub(crate) resolved_language: String,
    pub(crate) dialect: Option<String>,
    pub(crate) primary_context_id: Option<String>,
    pub(crate) secondary_context_ids: Vec<String>,
    pub(crate) frameworks: Vec<String>,
    pub(crate) runtime_profile: Option<String>,
    pub(crate) classification_reason: String,
    pub(crate) context_fingerprint: Option<String>,
    pub(crate) extraction_fingerprint: String,
    pub(crate) excluded_by_policy: bool,
}

impl ResolvedFileClassification {
    pub(crate) fn should_persist_current_state(&self) -> bool {
        self.analysis_mode != AnalysisMode::Excluded
    }

    pub(crate) fn should_extract(&self) -> bool {
        matches!(self.analysis_mode, AnalysisMode::Code)
            || (self.analysis_mode == AnalysisMode::Text
                && self.text_index_mode != TextIndexMode::None)
    }
}
