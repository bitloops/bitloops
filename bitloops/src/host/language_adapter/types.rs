use serde_json::{Value, json};

use crate::host::devql::{CallForm, EdgeKind, ExportForm, ImportForm, RefKind, Resolution};

use super::LanguageKind;

#[derive(Debug, Clone)]
pub(crate) struct LanguageArtefact {
    pub(crate) canonical_kind: Option<String>,
    pub(crate) language_kind: LanguageKind,
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

pub(crate) fn normalize_artefact_signature(signature: &str) -> String {
    let mut normalized = signature.trim();

    if let Some(without_closing_brace) = normalized.strip_suffix('}') {
        let candidate = without_closing_brace.trim_end();
        if let Some(without_empty_body) = candidate.strip_suffix('{') {
            normalized = without_empty_body.trim_end();
        }
    }

    if let Some(without_opening_brace) = normalized.strip_suffix('{') {
        normalized = without_opening_brace.trim_end();
    }

    normalized.to_string()
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

#[derive(Debug, Clone, PartialEq)]
pub struct LanguageHttpFactFile {
    pub(crate) repo_id: String,
    pub(crate) path: String,
    pub(crate) language: String,
    pub(crate) content_id: String,
    pub(crate) parser_version: String,
    pub(crate) extractor_version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LanguageHttpFactArtefact {
    pub(crate) symbol_id: String,
    pub(crate) artefact_id: String,
    pub(crate) symbol_fqn: String,
    pub(crate) canonical_kind: Option<String>,
    pub(crate) language_kind: String,
    pub(crate) start_line: i32,
    pub(crate) end_line: i32,
    pub(crate) start_byte: i32,
    pub(crate) end_byte: i32,
    pub(crate) signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LanguageHttpFactEvidence {
    pub(crate) path: String,
    pub(crate) artefact_id: Option<String>,
    pub(crate) symbol_id: Option<String>,
    pub(crate) content_id: String,
    pub(crate) start_line: Option<i32>,
    pub(crate) end_line: Option<i32>,
    pub(crate) start_byte: Option<i32>,
    pub(crate) end_byte: Option<i32>,
    pub(crate) properties: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LanguageHttpFact {
    pub(crate) stable_key: String,
    pub(crate) primitive_type: String,
    pub(crate) subject: String,
    pub(crate) roles: Vec<String>,
    pub(crate) terms: Vec<String>,
    pub(crate) properties: Value,
    pub(crate) confidence_level: String,
    pub(crate) confidence_score: f64,
    pub(crate) evidence: Vec<LanguageHttpFactEvidence>,
}

#[derive(Debug)]
pub(crate) struct RustUseExportEntry {
    pub(crate) path: String,
    pub(crate) export_name: String,
}

#[cfg(test)]
mod tests {
    use super::normalize_artefact_signature;

    #[test]
    fn normalize_artefact_signature_removes_trailing_body_openers() {
        assert_eq!(
            normalize_artefact_signature("export function hello() {"),
            "export function hello()"
        );
        assert_eq!(normalize_artefact_signature("func Run() {"), "func Run()");
        assert_eq!(
            normalize_artefact_signature("impl Repo for PgRepo {"),
            "impl Repo for PgRepo"
        );
    }

    #[test]
    fn normalize_artefact_signature_removes_empty_inline_bodies() {
        assert_eq!(
            normalize_artefact_signature("export class Widget {}"),
            "export class Widget"
        );
        assert_eq!(
            normalize_artefact_signature("render(): void { }"),
            "render(): void"
        );
    }

    #[test]
    fn normalize_artefact_signature_preserves_non_trailing_braces() {
        assert_eq!(
            normalize_artefact_signature("struct User { id: u64 }"),
            "struct User { id: u64 }"
        );
    }
}
