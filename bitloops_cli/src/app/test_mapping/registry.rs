use std::path::Path;

use anyhow::Result;

use crate::app::test_mapping::languages::{
    rust::RustLanguageProvider, typescript::TypeScriptLanguageProvider,
};
use crate::app::test_mapping::model::{DiscoveredTestFile, EnumerationResult, ReconciledDiscovery};

pub(crate) trait LanguageProvider {
    fn language_id(&self) -> &'static str;
    fn priority(&self) -> u8;
    fn supports_path(&self, absolute_path: &Path, relative_path: &str) -> bool;
    fn discover_tests(
        &mut self,
        absolute_path: &Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile>;

    fn enumerate_tests(&mut self, _repo_dir: &Path) -> EnumerationResult {
        EnumerationResult::default()
    }

    fn reconcile(
        &self,
        _source_files: &[DiscoveredTestFile],
        enumeration: EnumerationResult,
    ) -> ReconciledDiscovery {
        ReconciledDiscovery {
            enumerated_scenarios: enumeration.scenarios,
        }
    }
}

pub(crate) struct StructuralMappingRegistry {
    providers: Vec<Box<dyn LanguageProvider>>,
}

impl StructuralMappingRegistry {
    pub(crate) fn new() -> Result<Self> {
        Ok(Self {
            providers: vec![
                Box::new(RustLanguageProvider::new()?),
                Box::new(TypeScriptLanguageProvider::new()?),
            ],
        })
    }

    pub(crate) fn providers(&self) -> &[Box<dyn LanguageProvider>] {
        &self.providers
    }

    pub(crate) fn provider_mut(&mut self, index: usize) -> &mut dyn LanguageProvider {
        &mut *self.providers[index]
    }

    pub(crate) fn enumerate_all(
        &mut self,
        repo_dir: &Path,
    ) -> Vec<(&'static str, EnumerationResult)> {
        self.providers
            .iter_mut()
            .map(|provider| (provider.language_id(), provider.enumerate_tests(repo_dir)))
            .collect()
    }
}
