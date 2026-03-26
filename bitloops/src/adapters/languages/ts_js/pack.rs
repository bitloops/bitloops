use anyhow::Result;

use crate::host::extension_host::builtins::TS_JS_LANGUAGE_PACK;
use crate::host::extension_host::LanguagePackDescriptor;
use crate::host::language_adapter::{
    CanonicalMapping, DependencyEdge, LanguageAdapterPack, LanguageArtefact,
};

use super::canonical::{TS_JS_CANONICAL_MAPPINGS, TS_JS_SUPPORTED_LANGUAGE_KINDS};
use super::edges::extract_js_ts_dependency_edges;
use super::extraction::extract_js_ts_artefacts;

pub(crate) struct TsJsLanguageAdapterPack;

impl LanguageAdapterPack for TsJsLanguageAdapterPack {
    fn descriptor(&self) -> &'static LanguagePackDescriptor {
        &TS_JS_LANGUAGE_PACK
    }

    fn canonical_mappings(&self) -> &'static [CanonicalMapping] {
        TS_JS_CANONICAL_MAPPINGS
    }

    fn supported_language_kinds(&self) -> &'static [&'static str] {
        TS_JS_SUPPORTED_LANGUAGE_KINDS
    }

    fn extract_artefacts(&self, content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
        extract_js_ts_artefacts(content, path)
    }

    fn extract_dependency_edges(
        &self,
        content: &str,
        path: &str,
        artefacts: &[LanguageArtefact],
    ) -> Result<Vec<DependencyEdge>> {
        extract_js_ts_dependency_edges(content, path, artefacts)
    }
}
