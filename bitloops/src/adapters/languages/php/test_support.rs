use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::host::language_adapter::{
    DiscoveredTestFile, DiscoveredTestScenario, DiscoveredTestSuite, LanguageTestSupport,
    ReferenceCandidate, ScenarioDiscoverySource,
};

#[derive(Default)]
pub(crate) struct PhpLanguageTestSupport;

impl LanguageTestSupport for PhpLanguageTestSupport {
    fn language_id(&self) -> &'static str {
        "php"
    }

    fn priority(&self) -> u8 {
        2
    }

    fn supports_path(&self, _absolute_path: &Path, relative_path: &str) -> bool {
        relative_path.ends_with("Test.php")
            || relative_path.ends_with("Tests.php")
            || relative_path.contains("/tests/") && relative_path.ends_with(".php")
    }

    fn discover_tests(
        &self,
        absolute_path: &Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        let source = std::fs::read_to_string(absolute_path)?;
        let scenarios = source
            .lines()
            .enumerate()
            .filter_map(|(idx, line)| {
                let trimmed = line.trim();
                if !(trimmed.starts_with("public function test")
                    || trimmed.starts_with("function test")
                    || trimmed.contains("it('")
                    || trimmed.contains("test('"))
                {
                    return None;
                }

                Some(DiscoveredTestScenario {
                    name: trimmed.to_string(),
                    start_line: idx as i64 + 1,
                    end_line: idx as i64 + 1,
                    reference_candidates: Vec::<ReferenceCandidate>::new(),
                    discovery_source: ScenarioDiscoverySource::Source,
                })
            })
            .collect::<Vec<_>>();

        Ok(DiscoveredTestFile {
            relative_path: relative_path.to_string(),
            language: "php".to_string(),
            reference_candidates: Vec::new(),
            suites: vec![DiscoveredTestSuite {
                name: relative_path.to_string(),
                start_line: 1,
                end_line: source.lines().count() as i64,
                scenarios,
            }],
        })
    }
}

pub(crate) fn php_test_support() -> Arc<dyn LanguageTestSupport> {
    Arc::new(PhpLanguageTestSupport)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_php_test_paths() {
        let support = PhpLanguageTestSupport;
        assert!(support.supports_path(Path::new("tests/UserTest.php"), "tests/UserTest.php"));
        assert!(support.supports_path(Path::new("app/tests/Foo.php"), "app/tests/Foo.php"));
        assert!(!support.supports_path(Path::new("src/UserService.php"), "src/UserService.php"));
    }
}
