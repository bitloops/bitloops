use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::host::language_adapter::{
    DiscoveredTestFile, DiscoveredTestScenario, DiscoveredTestSuite, LanguageTestSupport,
    ReferenceCandidate, ScenarioDiscoverySource,
};

pub(crate) fn cpp_test_support() -> Arc<dyn LanguageTestSupport> {
    Arc::new(CppTestSupport)
}

struct CppTestSupport;

impl LanguageTestSupport for CppTestSupport {
    fn language_id(&self) -> &'static str {
        "cpp"
    }

    fn priority(&self) -> u8 {
        80
    }

    fn supports_path(&self, _absolute_path: &Path, relative_path: &str) -> bool {
        supports_cpp_test_path(relative_path)
    }

    fn discover_tests(
        &self,
        absolute_path: &Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        let content = std::fs::read_to_string(absolute_path)?;
        Ok(discover_cpp_tests_from_source(relative_path, &content))
    }
}

fn supports_cpp_test_path(relative_path: &str) -> bool {
    let lower = relative_path.to_ascii_lowercase();
    lower.ends_with("_test.cpp")
        || lower.ends_with("_test.cc")
        || lower.ends_with("_test.cxx")
        || (lower.contains("/tests/") && has_cpp_source_extension(&lower))
}

fn has_cpp_source_extension(relative_path: &str) -> bool {
    matches!(
        Path::new(relative_path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "hxx")
    )
}

fn discover_cpp_tests_from_source(relative_path: &str, content: &str) -> DiscoveredTestFile {
    let mut scenarios = Vec::new();

    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(args) =
            extract_macro_args(trimmed, "TEST").or_else(|| extract_macro_args(trimmed, "TEST_F"))
        {
            let mut parts = args.split(',').map(str::trim);
            let suite = parts.next().unwrap_or("suite");
            let scenario = parts.next().unwrap_or("case");
            scenarios.push(DiscoveredTestScenario {
                name: format!("{suite}.{scenario}"),
                start_line: index as i64 + 1,
                end_line: index as i64 + 1,
                reference_candidates: vec![ReferenceCandidate::SymbolName(scenario.to_string())],
                discovery_source: ScenarioDiscoverySource::Source,
            });
        }
    }

    let suites = if scenarios.is_empty() {
        Vec::new()
    } else {
        vec![DiscoveredTestSuite {
            name: "cpp_tests".to_string(),
            start_line: scenarios
                .first()
                .map(|scenario| scenario.start_line)
                .unwrap_or(1),
            end_line: scenarios
                .last()
                .map(|scenario| scenario.end_line)
                .unwrap_or(1),
            scenarios,
        }]
    };

    DiscoveredTestFile {
        relative_path: relative_path.to_string(),
        language: "cpp".to_string(),
        reference_candidates: Vec::new(),
        suites,
    }
}

fn extract_macro_args<'a>(line: &'a str, macro_name: &str) -> Option<&'a str> {
    let prefix = format!("{macro_name}(");
    let raw = line.strip_prefix(&prefix)?;
    let end = raw.find(')')?;
    Some(&raw[..end])
}

#[cfg(test)]
mod tests {
    use super::{discover_cpp_tests_from_source, supports_cpp_test_path};

    #[test]
    fn cpp_test_support_recognizes_test_paths() {
        assert!(supports_cpp_test_path("tests/user_service_test.cpp"));
        assert!(supports_cpp_test_path("src/module/component_test.cc"));
        assert!(!supports_cpp_test_path("tests/fixtures/snapshot.png"));
        assert!(!supports_cpp_test_path("src/main.cpp"));
    }

    #[test]
    fn cpp_test_support_discovers_gtest_macros() {
        let discovered = discover_cpp_tests_from_source(
            "tests/sample_test.cpp",
            r#"
TEST(UserServiceTest, ReturnsUser) {
}

TEST_F(UserServiceFixture, SavesUser) {
}
"#,
        );

        assert_eq!(discovered.language, "cpp");
        assert_eq!(discovered.suites.len(), 1);
        assert_eq!(discovered.suites[0].scenarios.len(), 2);
        assert_eq!(
            discovered.suites[0].scenarios[0].name,
            "UserServiceTest.ReturnsUser"
        );
        assert_eq!(
            discovered.suites[0].scenarios[1].name,
            "UserServiceFixture.SavesUser"
        );
    }
}
