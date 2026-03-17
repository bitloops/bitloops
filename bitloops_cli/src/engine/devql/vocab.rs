#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeKind {
    Imports,
    Calls,
    References,
    Extends,
    Implements,
    Exports,
}

impl EdgeKind {
    const ALL: [Self; 6] = [
        Self::Imports,
        Self::Calls,
        Self::References,
        Self::Extends,
        Self::Implements,
        Self::Exports,
    ];

    const LEGACY_INHERITS: &'static str = "inherits";

    fn as_str(self) -> &'static str {
        match self {
            Self::Imports => "imports",
            Self::Calls => "calls",
            Self::References => "references",
            Self::Extends => "extends",
            Self::Implements => "implements",
            Self::Exports => "exports",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "imports" => Some(Self::Imports),
            "calls" => Some(Self::Calls),
            "references" => Some(Self::References),
            "extends" | Self::LEGACY_INHERITS => Some(Self::Extends),
            "implements" => Some(Self::Implements),
            "exports" => Some(Self::Exports),
            _ => None,
        }
    }

    fn all_names() -> Vec<&'static str> {
        Self::ALL.into_iter().map(Self::as_str).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DepsKind {
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

    fn as_str(self) -> &'static str {
        match self {
            Self::Imports => EdgeKind::Imports.as_str(),
            Self::Calls => EdgeKind::Calls.as_str(),
            Self::References => EdgeKind::References.as_str(),
            Self::Extends => EdgeKind::Extends.as_str(),
            Self::Implements => EdgeKind::Implements.as_str(),
            Self::Exports => EdgeKind::Exports.as_str(),
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match EdgeKind::from_str(value)? {
            EdgeKind::Imports => Some(Self::Imports),
            EdgeKind::Calls => Some(Self::Calls),
            EdgeKind::References => Some(Self::References),
            EdgeKind::Extends => Some(Self::Extends),
            EdgeKind::Implements => Some(Self::Implements),
            EdgeKind::Exports => Some(Self::Exports),
        }
    }

    fn all_names() -> Vec<&'static str> {
        Self::ALL.into_iter().map(Self::as_str).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DepsDirection {
    Out,
    In,
    Both,
}

impl DepsDirection {
    const ALL: [Self; 3] = [Self::Out, Self::In, Self::Both];

    fn as_str(self) -> &'static str {
        match self {
            Self::Out => "out",
            Self::In => "in",
            Self::Both => "both",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "out" => Some(Self::Out),
            "in" => Some(Self::In),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    fn all_names() -> Vec<&'static str> {
        Self::ALL.into_iter().map(Self::as_str).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportForm {
    Binding,
    SideEffect,
}

impl ImportForm {
    fn as_str(self) -> &'static str {
        match self {
            Self::Binding => "binding",
            Self::SideEffect => "side_effect",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefKind {
    Type,
    Value,
}

impl RefKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Value => "value",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallForm {
    Identifier,
    Member,
    Function,
    Associated,
    Method,
    Macro,
}

impl CallForm {
    fn as_str(self) -> &'static str {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolution {
    Local,
    Import,
    Unresolved,
    ReExport,
    External,
}

impl Resolution {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Import => "import",
            Self::Unresolved => "unresolved",
            Self::ReExport => "re_export",
            Self::External => "external",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportForm {
    Declaration,
    Default,
    Named,
    ReExport,
    ReExportAll,
    ReExportNamespace,
    PubUse,
}

impl ExportForm {
    fn as_str(self) -> &'static str {
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

fn normalise_edge_kind_value(value: &str) -> Option<String> {
    EdgeKind::from_str(value).map(|kind| kind.as_str().to_string())
}

fn normalise_edge_metadata(edge_kind: &str, metadata: &mut Value) {
    let Some(obj) = metadata.as_object_mut() else {
        return;
    };

    obj.remove("inherit_form");
    obj.remove("relation");

    if let Some(Value::String(import_form)) = obj.get_mut("import_form") {
        let normalized = match import_form.as_str() {
            "module" | "use" | "binding" => ImportForm::Binding.as_str(),
            "side_effect" => ImportForm::SideEffect.as_str(),
            other => other,
        };
        *import_form = normalized.to_string();
    }

    match EdgeKind::from_str(edge_kind) {
        Some(EdgeKind::Extends) | Some(EdgeKind::Implements) => {
            obj.clear();
        }
        _ => {}
    }
}
