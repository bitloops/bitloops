use anyhow::Result;

use super::canonical::{CPP_CANONICAL_MAPPINGS, CPP_SUPPORTED_LANGUAGE_KINDS};
use super::edges::extract_cpp_dependency_edges;
use super::extraction::extract_cpp_artefacts;
use super::test_support::cpp_test_support;
use crate::host::extension_host::LanguagePackDescriptor;
use crate::host::extension_host::builtins::CPP_LANGUAGE_PACK;
use crate::host::language_adapter::{
    CanonicalMapping, DependencyEdge, LanguageAdapterPack, LanguageArtefact, LanguageKind,
    LanguageTestSupport,
};
use std::sync::Arc;

pub(crate) struct CppLanguageAdapterPack;

impl LanguageAdapterPack for CppLanguageAdapterPack {
    fn descriptor(&self) -> &'static LanguagePackDescriptor {
        &CPP_LANGUAGE_PACK
    }

    fn canonical_mappings(&self) -> &'static [CanonicalMapping] {
        CPP_CANONICAL_MAPPINGS
    }

    fn supported_language_kinds(&self) -> &'static [LanguageKind] {
        CPP_SUPPORTED_LANGUAGE_KINDS
    }

    fn extract_artefacts(&self, content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
        extract_cpp_artefacts(content, path)
    }

    fn extract_dependency_edges(
        &self,
        content: &str,
        path: &str,
        artefacts: &[LanguageArtefact],
    ) -> Result<Vec<DependencyEdge>> {
        extract_cpp_dependency_edges(content, path, artefacts)
    }

    fn test_support(&self) -> Option<Arc<dyn LanguageTestSupport>> {
        Some(cpp_test_support())
    }
}
