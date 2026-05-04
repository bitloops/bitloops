use anyhow::Result;
use std::sync::Arc;

use crate::host::extension_host::LanguagePackDescriptor;
use crate::host::extension_host::builtins::TS_JS_LANGUAGE_PACK;
use crate::host::language_adapter::{
    BuiltinEntryPointLanguage, BuiltinLanguageEntryPointSupport, CanonicalMapping, DependencyEdge,
    LanguageAdapterPack, LanguageArtefact, LanguageEntryPointSupport, LanguageKind,
    LanguageTestSupport,
};

use super::canonical::{TS_JS_CANONICAL_MAPPINGS, TS_JS_SUPPORTED_LANGUAGE_KINDS};
use super::edges::extract_js_ts_dependency_edges;
use super::extraction::extract_js_ts_artefacts;
use super::test_support::ts_js_test_support;

pub(crate) struct TsJsLanguageAdapterPack;

impl LanguageAdapterPack for TsJsLanguageAdapterPack {
    fn descriptor(&self) -> &'static LanguagePackDescriptor {
        &TS_JS_LANGUAGE_PACK
    }

    fn canonical_mappings(&self) -> &'static [CanonicalMapping] {
        TS_JS_CANONICAL_MAPPINGS
    }

    fn supported_language_kinds(&self) -> &'static [LanguageKind] {
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

    fn test_support(&self) -> Option<Arc<dyn LanguageTestSupport>> {
        Some(ts_js_test_support())
    }

    fn entry_point_support(&self) -> Option<Arc<dyn LanguageEntryPointSupport>> {
        Some(Arc::new(BuiltinLanguageEntryPointSupport::new(
            BuiltinEntryPointLanguage::TsJs,
        )))
    }
}
