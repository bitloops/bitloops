use anyhow::Result;
use std::sync::Arc;

use super::canonical::{GO_CANONICAL_MAPPINGS, GO_SUPPORTED_LANGUAGE_KINDS};
use super::edges::extract_go_dependency_edges;
use super::extraction::extract_go_artefacts;
use super::test_support::go_test_support;
use crate::host::extension_host::LanguagePackDescriptor;
use crate::host::extension_host::builtins::GO_LANGUAGE_PACK;
use crate::host::language_adapter::{
    CanonicalMapping, DependencyEdge, LanguageAdapterPack, LanguageArtefact, LanguageTestSupport,
};

pub(crate) struct GoLanguageAdapterPack;

impl LanguageAdapterPack for GoLanguageAdapterPack {
    fn descriptor(&self) -> &'static LanguagePackDescriptor {
        &GO_LANGUAGE_PACK
    }

    fn canonical_mappings(&self) -> &'static [CanonicalMapping] {
        GO_CANONICAL_MAPPINGS
    }

    fn supported_language_kinds(&self) -> &'static [&'static str] {
        GO_SUPPORTED_LANGUAGE_KINDS
    }

    fn extract_artefacts(&self, content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
        extract_go_artefacts(content, path)
    }

    fn extract_dependency_edges(
        &self,
        content: &str,
        path: &str,
        artefacts: &[LanguageArtefact],
    ) -> Result<Vec<DependencyEdge>> {
        extract_go_dependency_edges(content, path, artefacts)
    }

    fn test_support(&self) -> Option<Arc<dyn LanguageTestSupport>> {
        Some(go_test_support())
    }
}
