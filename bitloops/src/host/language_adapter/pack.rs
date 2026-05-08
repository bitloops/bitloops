use anyhow::Result;
use std::sync::Arc;

use super::{
    CanonicalMapping, DependencyEdge, LanguageAdapterHealthCheck,
    LanguageAdapterMigrationDescriptor, LanguageArtefact, LanguageEntryPointSupport,
    LanguageHttpFact, LanguageHttpFactArtefact, LanguageHttpFactFile, LanguageKind,
    LanguageTestSupport,
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

    fn extract_http_facts(
        &self,
        file: &LanguageHttpFactFile,
        content: &str,
        artefacts: &[LanguageHttpFactArtefact],
    ) -> Result<Vec<LanguageHttpFact>> {
        let _ = (file, content, artefacts);
        Ok(Vec::new())
    }

    fn test_support(&self) -> Option<Arc<dyn LanguageTestSupport>> {
        None
    }

    fn entry_point_support(&self) -> Option<Arc<dyn LanguageEntryPointSupport>> {
        None
    }

    fn migrations(&self) -> &'static [LanguageAdapterMigrationDescriptor] {
        &[]
    }

    fn health_checks(&self) -> &'static [LanguageAdapterHealthCheck] {
        &[]
    }
}
