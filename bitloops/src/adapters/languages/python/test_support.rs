use std::sync::Arc;

use anyhow::Result;

use crate::capability_packs::test_harness::mapping::languages::python::PythonLanguageProvider;
use crate::capability_packs::test_harness::mapping::registry::LanguageProvider;
use crate::host::language_adapter::{DiscoveredTestFile, LanguageTestSupport};

#[derive(Default)]
pub(crate) struct PythonLanguageTestSupport;

impl LanguageTestSupport for PythonLanguageTestSupport {
    fn language_id(&self) -> &'static str {
        "python"
    }

    fn priority(&self) -> u8 {
        2
    }

    fn supports_path(&self, absolute_path: &std::path::Path, relative_path: &str) -> bool {
        PythonLanguageProvider::new()
            .map(|provider| provider.supports_path(absolute_path, relative_path))
            .unwrap_or(false)
    }

    fn discover_tests(
        &self,
        absolute_path: &std::path::Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        PythonLanguageProvider::new()?.discover_tests(absolute_path, relative_path)
    }
}

pub(crate) fn python_test_support() -> Arc<dyn LanguageTestSupport> {
    Arc::new(PythonLanguageTestSupport)
}
