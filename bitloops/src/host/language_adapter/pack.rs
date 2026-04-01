use anyhow::Result;
use std::sync::Arc;

use super::{
    CanonicalMapping, DependencyEdge, LanguageAdapterHealthCheck,
    LanguageAdapterMigrationDescriptor, LanguageArtefact, LanguageKind, LanguageTestSupport,
};
use crate::host::extension_host::LanguagePackDescriptor;

pub(crate) trait LanguageAdapterPack: Send + Sync {
    fn descriptor(&self) -> &'static LanguagePackDescriptor;
    fn canonical_mappings(&self) -> &'static [CanonicalMapping];
    fn supported_language_kinds(&self) -> &'static [LanguageKind];
    fn extract_artefacts(&self, content: &str, path: &str) -> Result<Vec<LanguageArtefact>>;
    fn extract_dependency_edges(
        &self,
        content: &str,
        path: &str,
        artefacts: &[LanguageArtefact],
    ) -> Result<Vec<DependencyEdge>>;
    fn extract_file_docstring(&self, content: &str) -> Option<String> {
        let _ = content;
        None
    }

    fn test_support(&self) -> Option<Arc<dyn LanguageTestSupport>> {
        None
    }

    fn migrations(&self) -> &'static [LanguageAdapterMigrationDescriptor] {
        &[]
    }

    fn health_checks(&self) -> &'static [LanguageAdapterHealthCheck] {
        &[]
    }
}
