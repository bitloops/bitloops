use serde_json::{json, Value};

use crate::host::devql::{
    CallForm, EdgeKind, ExportForm, ImportForm, RefKind, Resolution,
};

#[derive(Debug, Clone)]
pub(crate) struct LanguageArtefact {
    pub(crate) canonical_kind: Option<String>,
    pub(crate) language_kind: String,
    pub(crate) name: String,
    pub(crate) symbol_fqn: String,
    pub(crate) parent_symbol_fqn: Option<String>,
    pub(crate) start_line: i32,
    pub(crate) end_line: i32,
    pub(crate) start_byte: i32,
    pub(crate) end_byte: i32,
    pub(crate) signature: String,
    pub(crate) modifiers: Vec<String>,
    pub(crate) docstring: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct EdgeMetadata(pub(crate) Value);

impl EdgeMetadata {
    pub(crate) fn none() -> Self {
        Self(json!({}))
    }

    pub(crate) fn import(import_form: ImportForm) -> Self {
        Self(json!({
            "import_form": import_form.as_str(),
        }))
    }

    pub(crate) fn call(call_form: CallForm, resolution: Resolution) -> Self {
        Self(json!({
            "call_form": call_form.as_str(),
            "resolution": resolution.as_str(),
        }))
    }

    pub(crate) fn reference(ref_kind: RefKind, resolution: Resolution) -> Self {
        Self(json!({
            "ref_kind": ref_kind.as_str(),
            "resolution": resolution.as_str(),
        }))
    }

    pub(crate) fn export(
        export_name: String,
        export_form: ExportForm,
        resolution: Resolution,
    ) -> Self {
        Self(json!({
            "export_name": export_name,
            "export_form": export_form.as_str(),
            "resolution": resolution.as_str(),
        }))
    }

    pub(crate) fn to_value(&self) -> Value {
        self.0.clone()
    }
}

impl std::ops::Deref for EdgeMetadata {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DependencyEdge {
    pub(crate) edge_kind: EdgeKind,
    pub(crate) from_symbol_fqn: String,
    pub(crate) to_target_symbol_fqn: Option<String>,
    pub(crate) to_symbol_ref: Option<String>,
    pub(crate) start_line: Option<i32>,
    pub(crate) end_line: Option<i32>,
    pub(crate) metadata: EdgeMetadata,
}

#[derive(Debug)]
pub(crate) struct RustUseExportEntry {
    pub(crate) path: String,
    pub(crate) export_name: String,
}
