use std::sync::Arc;

use anyhow::Result;

use crate::capability_packs::test_harness::mapping::languages::typescript::TypeScriptLanguageProvider;
use crate::capability_packs::test_harness::mapping::registry::LanguageProvider;
use crate::host::language_adapter::{DiscoveredTestFile, LanguageTestSupport};

#[derive(Default)]
pub(crate) struct TsJsLanguageTestSupport;

impl LanguageTestSupport for TsJsLanguageTestSupport {
    fn language_id(&self) -> &'static str {
        "typescript"
    }

    fn priority(&self) -> u8 {
        1
    }

    fn supports_path(&self, absolute_path: &std::path::Path, relative_path: &str) -> bool {
        TypeScriptLanguageProvider::new()
            .map(|provider| provider.supports_path(absolute_path, relative_path))
            .unwrap_or(false)
    }

    fn discover_tests(
        &self,
        absolute_path: &std::path::Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        TypeScriptLanguageProvider::new()?.discover_tests(absolute_path, relative_path)
    }
}

pub(crate) fn ts_js_test_support() -> Arc<dyn LanguageTestSupport> {
    Arc::new(TsJsLanguageTestSupport)
}
