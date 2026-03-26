use anyhow::Result;

use super::{CanonicalMapping, DependencyEdge, LanguageArtefact};
use crate::host::extension_host::LanguagePackDescriptor;

pub(crate) trait LanguageAdapterPack: Send + Sync {
    fn descriptor(&self) -> &'static LanguagePackDescriptor;
    fn canonical_mappings(&self) -> &'static [CanonicalMapping];
    fn supported_language_kinds(&self) -> &'static [&'static str];
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
}
