use anyhow::Result;
use std::sync::Arc;

use super::canonical::{RUST_CANONICAL_MAPPINGS, RUST_SUPPORTED_LANGUAGE_KINDS};
use super::edges::extract_rust_dependency_edges;
use super::extraction::{extract_rust_artefacts, extract_rust_file_docstring};
use super::test_support::rust_test_support;
use crate::host::extension_host::LanguagePackDescriptor;
use crate::host::language_adapter::{
    CanonicalMapping, DependencyEdge, LanguageAdapterPack, LanguageArtefact, LanguageKind,
    LanguageTestSupport,
};

pub(crate) struct RustLanguageAdapterPack;

impl LanguageAdapterPack for RustLanguageAdapterPack {
    fn descriptor(&self) -> &'static LanguagePackDescriptor {
        &crate::host::extension_host::builtins::RUST_LANGUAGE_PACK
    }

    fn canonical_mappings(&self) -> &'static [CanonicalMapping] {
        RUST_CANONICAL_MAPPINGS
    }

    fn supported_language_kinds(&self) -> &'static [LanguageKind] {
        RUST_SUPPORTED_LANGUAGE_KINDS
    }

    fn extract_artefacts(&self, content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
        extract_rust_artefacts(content, path)
    }

    fn extract_dependency_edges(
        &self,
        content: &str,
        path: &str,
        artefacts: &[LanguageArtefact],
    ) -> Result<Vec<DependencyEdge>> {
        extract_rust_dependency_edges(content, path, artefacts)
    }

    fn extract_file_docstring(&self, content: &str) -> Option<String> {
        extract_rust_file_docstring(content)
    }

    fn test_support(&self) -> Option<Arc<dyn LanguageTestSupport>> {
        Some(rust_test_support())
    }
}
