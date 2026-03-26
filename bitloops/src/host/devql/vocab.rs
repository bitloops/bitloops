use super::*;

pub(crate) const EDGE_KIND_IMPORTS: &str = "imports";
pub(crate) const EDGE_KIND_CALLS: &str = "calls";
pub(crate) const EDGE_KIND_REFERENCES: &str = "references";
pub(crate) const EDGE_KIND_EXTENDS: &str = "extends";
pub(crate) const EDGE_KIND_IMPLEMENTS: &str = "implements";
pub(crate) const EDGE_KIND_EXPORTS: &str = "exports";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum EdgeKind {
    Imports,
    Calls,
    References,
    Extends,
    Implements,
    Exports,
}

impl EdgeKind {
    const LEGACY_INHERITS: &'static str = "inherits";

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Imports => EDGE_KIND_IMPORTS,
            Self::Calls => EDGE_KIND_CALLS,
            Self::References => EDGE_KIND_REFERENCES,
            Self::Extends => EDGE_KIND_EXTENDS,
            Self::Implements => EDGE_KIND_IMPLEMENTS,
            Self::Exports => EDGE_KIND_EXPORTS,
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            EDGE_KIND_IMPORTS => Some(Self::Imports),
            EDGE_KIND_CALLS => Some(Self::Calls),
            EDGE_KIND_REFERENCES => Some(Self::References),
            EDGE_KIND_EXTENDS | Self::LEGACY_INHERITS => Some(Self::Extends),
            EDGE_KIND_IMPLEMENTS => Some(Self::Implements),
            EDGE_KIND_EXPORTS => Some(Self::Exports),
            _ => None,
        }
    }
}

impl PartialEq<&str> for EdgeKind {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum DepsKind {
    Imports,
    Calls,
    References,
    Extends,
    Implements,
    Exports,
}

impl DepsKind {
    const ALL: [Self; 6] = [
        Self::Imports,
        Self::Calls,
        Self::References,
        Self::Extends,
        Self::Implements,
        Self::Exports,
    ];

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Imports => EdgeKind::Imports.as_str(),
            Self::Calls => EdgeKind::Calls.as_str(),
            Self::References => EdgeKind::References.as_str(),
            Self::Extends => EdgeKind::Extends.as_str(),
            Self::Implements => EdgeKind::Implements.as_str(),
            Self::Exports => EdgeKind::Exports.as_str(),
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match EdgeKind::from_str(value)? {
            EdgeKind::Imports => Some(Self::Imports),
            EdgeKind::Calls => Some(Self::Calls),
            EdgeKind::References => Some(Self::References),
            EdgeKind::Extends => Some(Self::Extends),
            EdgeKind::Implements => Some(Self::Implements),
            EdgeKind::Exports => Some(Self::Exports),
        }
    }

    pub(super) fn all_names() -> [&'static str; 6] {
        Self::ALL.map(Self::as_str)
    }
}

impl PartialEq<&str> for DepsKind {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum DepsDirection {
    Out,
    In,
    Both,
}

impl DepsDirection {
    const ALL: [Self; 3] = [Self::Out, Self::In, Self::Both];

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Out => "out",
            Self::In => "in",
            Self::Both => "both",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "out" => Some(Self::Out),
            "in" => Some(Self::In),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    pub(super) fn all_names() -> [&'static str; 3] {
        Self::ALL.map(Self::as_str)
    }
}

impl PartialEq<&str> for DepsDirection {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ImportForm {
    Binding,
    SideEffect,
}

impl ImportForm {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Binding => "binding",
            Self::SideEffect => "side_effect",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "module" | "use" | "binding" => Some(Self::Binding),
            "side_effect" => Some(Self::SideEffect),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum RefKind {
    Type,
    Value,
}

impl RefKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Value => "value",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum CallForm {
    Identifier,
    Member,
    Function,
    Associated,
    Method,
    Macro,
}

impl CallForm {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Identifier => "identifier",
            Self::Member => "member",
            Self::Function => "function",
            Self::Associated => "associated",
            Self::Method => "method",
            Self::Macro => "macro",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Resolution {
    Local,
    Import,
    Unresolved,
    ReExport,
    External,
}

impl Resolution {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Import => "import",
            Self::Unresolved => "unresolved",
            Self::ReExport => "re_export",
            Self::External => "external",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ExportForm {
    Declaration,
    Default,
    Named,
    ReExport,
    ReExportAll,
    ReExportNamespace,
    PubUse,
}

impl ExportForm {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Declaration => "declaration",
            Self::Default => "default",
            Self::Named => "named",
            Self::ReExport => "re_export",
            Self::ReExportAll => "re_export_all",
            Self::ReExportNamespace => "re_export_namespace",
            Self::PubUse => "pub_use",
        }
    }
}

pub(super) fn normalise_edge_kind_value(value: &str) -> Option<String> {
    EdgeKind::from_str(value).map(|kind| kind.as_str().to_string())
}

pub(super) fn normalise_edge_metadata(edge_kind: &str, metadata: &mut Value) {
    let Some(obj) = metadata.as_object_mut() else {
        return;
    };

    obj.remove("inherit_form");
    obj.remove("relation");

    if let Some(Value::String(import_form)) = obj.get_mut("import_form")
        && let Some(normalized) = ImportForm::from_str(import_form)
    {
        *import_form = normalized.as_str().to_string();
    }

    match EdgeKind::from_str(edge_kind) {
        Some(EdgeKind::Extends) | Some(EdgeKind::Implements) => {
            obj.clear();
        }
        _ => {}
    }
}
