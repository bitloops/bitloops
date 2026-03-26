use super::*;

// Core-owned canonical contracts shared across extraction, persistence, and query layers.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum CoreCanonicalArtefactKind {
    File,
    Namespace,
    Module,
    Import,
    Type,
    Callable,
    Value,
    Member,
    Parameter,
    TypeParameter,
    Alias,
}

impl CoreCanonicalArtefactKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Namespace => "namespace",
            Self::Module => "module",
            Self::Import => "import",
            Self::Type => "type",
            Self::Callable => "callable",
            Self::Value => "value",
            Self::Member => "member",
            Self::Parameter => "parameter",
            Self::TypeParameter => "type_parameter",
            Self::Alias => "alias",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "file" => Some(Self::File),
            "namespace" => Some(Self::Namespace),
            "module" => Some(Self::Module),
            "import" => Some(Self::Import),
            "type" => Some(Self::Type),
            "callable" => Some(Self::Callable),
            "value" => Some(Self::Value),
            "member" => Some(Self::Member),
            "parameter" => Some(Self::Parameter),
            "type_parameter" => Some(Self::TypeParameter),
            "alias" => Some(Self::Alias),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CanonicalKindProjection {
    File,
    Module,
    Import,
    Type,
    Interface,
    Enum,
    Function,
    Method,
    Variable,
}

impl CanonicalKindProjection {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Module => "module",
            Self::Import => "import",
            Self::Type => "type",
            Self::Interface => "interface",
            Self::Enum => "enum",
            Self::Function => "function",
            Self::Method => "method",
            Self::Variable => "variable",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "file" => Some(Self::File),
            "module" => Some(Self::Module),
            "import" => Some(Self::Import),
            "type" => Some(Self::Type),
            "interface" => Some(Self::Interface),
            "enum" => Some(Self::Enum),
            "function" => Some(Self::Function),
            "method" => Some(Self::Method),
            "variable" => Some(Self::Variable),
            _ => None,
        }
    }

    pub(super) fn core_kind(self) -> CoreCanonicalArtefactKind {
        match self {
            Self::File => CoreCanonicalArtefactKind::File,
            Self::Module => CoreCanonicalArtefactKind::Module,
            Self::Import => CoreCanonicalArtefactKind::Import,
            Self::Type | Self::Interface | Self::Enum => CoreCanonicalArtefactKind::Type,
            Self::Function | Self::Method => CoreCanonicalArtefactKind::Callable,
            Self::Variable => CoreCanonicalArtefactKind::Value,
        }
    }
}

pub(super) fn artefact_core_kind(
    canonical_kind: Option<&str>,
) -> Option<CoreCanonicalArtefactKind> {
    canonical_kind
        .and_then(CanonicalKindProjection::from_str)
        .map(CanonicalKindProjection::core_kind)
}

pub(super) fn artefact_has_core_kind(
    canonical_kind: Option<&str>,
    expected: CoreCanonicalArtefactKind,
) -> bool {
    artefact_core_kind(canonical_kind).is_some_and(|kind| kind == expected)
}

pub(super) fn canonical_kind_filter_sql(column: &str, requested_kind: &str) -> String {
    let kind = requested_kind.trim();
    if kind.is_empty() {
        return "1 = 0".to_string();
    }

    if let Some(projection) = CanonicalKindProjection::from_str(kind) {
        return format!("{column} = '{}'", esc_pg(projection.as_str()));
    }

    if let Some(core_kind) = CoreCanonicalArtefactKind::from_str(kind) {
        let mut values = match core_kind {
            CoreCanonicalArtefactKind::File => vec!["file"],
            CoreCanonicalArtefactKind::Namespace => vec!["namespace"],
            CoreCanonicalArtefactKind::Module => vec!["module"],
            CoreCanonicalArtefactKind::Import => vec!["import"],
            CoreCanonicalArtefactKind::Type => vec!["type", "interface", "enum"],
            CoreCanonicalArtefactKind::Callable => vec!["callable", "function", "method"],
            CoreCanonicalArtefactKind::Value => vec!["value", "variable"],
            CoreCanonicalArtefactKind::Member => vec!["member"],
            CoreCanonicalArtefactKind::Parameter => vec!["parameter"],
            CoreCanonicalArtefactKind::TypeParameter => vec!["type_parameter"],
            CoreCanonicalArtefactKind::Alias => vec!["alias"],
        };
        if !values.iter().any(|value| *value == core_kind.as_str()) {
            values.push(core_kind.as_str());
        }
        values.sort_unstable();
        values.dedup();
        if values.len() == 1 {
            return format!("{column} = '{}'", esc_pg(values[0]));
        }

        let comparisons = values
            .into_iter()
            .map(|value| format!("{column} = '{}'", esc_pg(value)))
            .collect::<Vec<_>>();
        return format!("({})", comparisons.join(" OR "));
    }

    format!("{column} = '{}'", esc_pg(kind))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum TemporalRevisionKind {
    Commit,
    Temporary,
}

impl TemporalRevisionKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Temporary => "temporary",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "commit" => Some(Self::Commit),
            "temporary" => Some(Self::Temporary),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct TemporalRevisionRef<'a> {
    pub(super) kind: TemporalRevisionKind,
    pub(super) id: &'a str,
    pub(super) temp_checkpoint_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct CanonicalProvenanceRef<'a> {
    pub(super) repo_id: &'a str,
    pub(super) blob_sha: &'a str,
    pub(super) commit_sha: Option<&'a str>,
    pub(super) path: Option<&'a str>,
    pub(super) extension_family: Option<&'a str>,
    pub(super) extension_id: Option<&'a str>,
    pub(super) operation_run_id: Option<&'a str>,
}

impl<'a> CanonicalProvenanceRef<'a> {
    pub(super) fn for_blob(repo_id: &'a str, blob_sha: &'a str) -> Self {
        Self {
            repo_id,
            blob_sha,
            commit_sha: None,
            path: None,
            extension_family: None,
            extension_id: None,
            operation_run_id: None,
        }
    }

    pub(super) fn with_source_anchor(mut self, commit_sha: &'a str, path: &'a str) -> Self {
        self.commit_sha = Some(commit_sha);
        self.path = Some(path);
        self
    }

    pub(super) fn artefact_identity_scope(self) -> String {
        format!("{}|{}", self.repo_id, self.blob_sha)
    }

    pub(super) fn temporal_identity_scope(self) -> Option<String> {
        let commit_sha = self.commit_sha?;
        let path = self.path?;
        Some(format!("{commit_sha}|{path}|{}", self.blob_sha))
    }
}
