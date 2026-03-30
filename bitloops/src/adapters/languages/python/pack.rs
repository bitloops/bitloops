use anyhow::Result;
use std::sync::Arc;

use super::canonical::{PYTHON_CANONICAL_MAPPINGS, PYTHON_SUPPORTED_LANGUAGE_KINDS};
use super::edges::extract_python_dependency_edges;
use super::extraction::{extract_python_artefacts, extract_python_file_docstring};
use super::test_support::python_test_support;
use crate::host::extension_host::LanguagePackDescriptor;
use crate::host::extension_host::builtins::PYTHON_LANGUAGE_PACK;
use crate::host::language_adapter::{
    CanonicalMapping, DependencyEdge, LanguageAdapterPack, LanguageArtefact, LanguageTestSupport,
};

pub(crate) struct PythonLanguageAdapterPack;

impl LanguageAdapterPack for PythonLanguageAdapterPack {
    fn descriptor(&self) -> &'static LanguagePackDescriptor {
        &PYTHON_LANGUAGE_PACK
    }

    fn canonical_mappings(&self) -> &'static [CanonicalMapping] {
        PYTHON_CANONICAL_MAPPINGS
    }

    fn supported_language_kinds(&self) -> &'static [&'static str] {
        PYTHON_SUPPORTED_LANGUAGE_KINDS
    }

    fn extract_artefacts(&self, content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
        extract_python_artefacts(content, path)
    }

    fn extract_dependency_edges(
        &self,
        content: &str,
        path: &str,
        artefacts: &[LanguageArtefact],
    ) -> Result<Vec<DependencyEdge>> {
        extract_python_dependency_edges(content, path, artefacts)
    }

    fn extract_file_docstring(&self, content: &str) -> Option<String> {
        extract_python_file_docstring(content)
    }

    fn test_support(&self) -> Option<Arc<dyn LanguageTestSupport>> {
        Some(python_test_support())
    }
}
