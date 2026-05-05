use anyhow::Result;
use std::sync::Arc;

use super::canonical::{JAVA_CANONICAL_MAPPINGS, JAVA_SUPPORTED_LANGUAGE_KINDS};
use super::edges::extract_java_dependency_edges;
use super::extraction::{extract_java_artefacts, extract_java_file_docstring};
use super::test_support::java_test_support;
use crate::host::extension_host::LanguagePackDescriptor;
use crate::host::extension_host::builtins::JAVA_LANGUAGE_PACK;
use crate::host::language_adapter::{
    BuiltinEntryPointLanguage, BuiltinLanguageEntryPointSupport, CanonicalMapping, DependencyEdge,
    LanguageAdapterPack, LanguageArtefact, LanguageEntryPointSupport, LanguageKind,
    LanguageTestSupport,
};

pub(crate) struct JavaLanguageAdapterPack;

impl LanguageAdapterPack for JavaLanguageAdapterPack {
    fn descriptor(&self) -> &'static LanguagePackDescriptor {
        &JAVA_LANGUAGE_PACK
    }

    fn canonical_mappings(&self) -> &'static [CanonicalMapping] {
        JAVA_CANONICAL_MAPPINGS
    }

    fn supported_language_kinds(&self) -> &'static [LanguageKind] {
        JAVA_SUPPORTED_LANGUAGE_KINDS
    }

    fn extract_artefacts(&self, content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
        extract_java_artefacts(content, path)
    }

    fn extract_dependency_edges(
        &self,
        content: &str,
        path: &str,
        artefacts: &[LanguageArtefact],
    ) -> Result<Vec<DependencyEdge>> {
        extract_java_dependency_edges(content, path, artefacts)
    }

    fn extract_file_docstring(&self, content: &str) -> Option<String> {
        extract_java_file_docstring(content)
    }

    fn test_support(&self) -> Option<Arc<dyn LanguageTestSupport>> {
        Some(java_test_support())
    }

    fn entry_point_support(&self) -> Option<Arc<dyn LanguageEntryPointSupport>> {
        Some(Arc::new(BuiltinLanguageEntryPointSupport::new(
            BuiltinEntryPointLanguage::Java,
        )))
    }
}
