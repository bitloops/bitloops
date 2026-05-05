use anyhow::Result;
use std::sync::Arc;

use super::canonical::{CSHARP_CANONICAL_MAPPINGS, CSHARP_SUPPORTED_LANGUAGE_KINDS};
use super::edges::extract_csharp_dependency_edges;
use super::extraction::extract_csharp_artefacts;
use super::test_support::csharp_test_support;
use crate::host::extension_host::LanguagePackDescriptor;
use crate::host::extension_host::builtins::CSHARP_LANGUAGE_PACK;
use crate::host::language_adapter::{
    BuiltinEntryPointLanguage, BuiltinLanguageEntryPointSupport, CanonicalMapping, DependencyEdge,
    LanguageAdapterPack, LanguageArtefact, LanguageEntryPointSupport, LanguageKind,
    LanguageTestSupport,
};

pub(crate) struct CSharpLanguageAdapterPack;

impl LanguageAdapterPack for CSharpLanguageAdapterPack {
    fn descriptor(&self) -> &'static LanguagePackDescriptor {
        &CSHARP_LANGUAGE_PACK
    }

    fn canonical_mappings(&self) -> &'static [CanonicalMapping] {
        CSHARP_CANONICAL_MAPPINGS
    }

    fn supported_language_kinds(&self) -> &'static [LanguageKind] {
        CSHARP_SUPPORTED_LANGUAGE_KINDS
    }

    fn extract_artefacts(&self, content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
        extract_csharp_artefacts(content, path)
    }

    fn extract_dependency_edges(
        &self,
        content: &str,
        path: &str,
        artefacts: &[LanguageArtefact],
    ) -> Result<Vec<DependencyEdge>> {
        extract_csharp_dependency_edges(content, path, artefacts)
    }

    fn test_support(&self) -> Option<Arc<dyn LanguageTestSupport>> {
        Some(csharp_test_support())
    }

    fn entry_point_support(&self) -> Option<Arc<dyn LanguageEntryPointSupport>> {
        Some(Arc::new(BuiltinLanguageEntryPointSupport::new(
            BuiltinEntryPointLanguage::CSharp,
        )))
    }
}
