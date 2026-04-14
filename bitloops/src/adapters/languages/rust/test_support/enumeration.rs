use crate::host::language_adapter::{
    EnumeratedTestScenario, ReferenceCandidate, ScenarioDiscoverySource,
};

pub(crate) fn parse_enumerated_doctests(output: &str) -> Vec<EnumeratedTestScenario> {
    let mut scenarios = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.ends_with(": test") || !trimmed.contains(" - ") {
            continue;
        }
        let Some((path, remainder)) = trimmed.trim_end_matches(": test").split_once(" - ") else {
            continue;
        };
        let Some((item_name, line_number)) = parse_doctest_descriptor(remainder) else {
            continue;
        };

        scenarios.push(EnumeratedTestScenario {
            language: "rust".to_string(),
            suite_name: format!("{}::doctests", path.replace('/', "::")),
            scenario_name: item_name.clone(),
            relative_path: path.to_string(),
            start_line: line_number,
            reference_candidates: vec![ReferenceCandidate::ExplicitTarget {
                path: path.to_string(),
                start_line: line_number,
            }],
            discovery_source: ScenarioDiscoverySource::Doctest,
        });
    }

    scenarios
}

pub(crate) fn parse_enumerated_host_tests(output: &str) -> Vec<EnumeratedTestScenario> {
    let mut scenarios = Vec::new();
    let mut current_host_context_path: Option<String> = None;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Doc-tests ") || trimmed.ends_with(" benchmarks") {
            current_host_context_path = None;
            continue;
        }
        if let Some(context_path) = parse_running_host_test_context_path(trimmed) {
            current_host_context_path = Some(context_path);
            continue;
        }
        if !trimmed.ends_with(": test") || trimmed.contains(" - ") {
            continue;
        }

        let name = trimmed.trim_end_matches(": test").trim();
        if name.is_empty() || name.starts_with("Doc-tests ") || name.starts_with("Running ") {
            continue;
        }
        let Some(relative_path) = current_host_context_path.clone() else {
            continue;
        };

        let segments: Vec<&str> = name.split("::").collect();
        let scenario_name = segments.last().copied().unwrap_or(name).to_string();
        let suite_name = if segments.len() > 1 {
            segments[..segments.len() - 1].join("::")
        } else {
            "enumerated".to_string()
        };

        scenarios.push(EnumeratedTestScenario {
            language: "rust".to_string(),
            suite_name,
            scenario_name: scenario_name.clone(),
            relative_path,
            start_line: 1,
            reference_candidates: vec![ReferenceCandidate::SymbolName(scenario_name)],
            discovery_source: ScenarioDiscoverySource::Enumeration,
        });
    }

    scenarios
}

fn parse_doctest_descriptor(raw: &str) -> Option<(String, i64)> {
    let (item_name, line_part) = raw.rsplit_once("(line ")?;
    let line_number = line_part.trim_end_matches(')').parse().ok()?;
    Some((item_name.trim().to_string(), line_number))
}

fn parse_running_host_test_context_path(line: &str) -> Option<String> {
    let descriptor = line.strip_prefix("Running ")?.trim();
    if descriptor.is_empty() || descriptor.starts_with("Doc-tests ") {
        return None;
    }

    Some(format!(
        "__synthetic_tests__/{}",
        sanitize_host_test_context_descriptor(descriptor)
    ))
}

fn sanitize_host_test_context_descriptor(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '/' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_enumerated_host_tests;

    #[test]
    fn host_enumeration_namespaces_duplicate_test_names_by_running_binary() {
        let output = r#"
Running unittests src/lib.rs (target/debug/deps/crate_a-1111111111111111)
main: test
Running tests/api.rs (target/debug/deps/api-2222222222222222)
main: test
"#;

        let scenarios = parse_enumerated_host_tests(output);
        assert_eq!(scenarios.len(), 2);
        assert_eq!(scenarios[0].suite_name, "enumerated");
        assert_eq!(scenarios[0].scenario_name, "main");
        assert_eq!(scenarios[1].suite_name, "enumerated");
        assert_eq!(scenarios[1].scenario_name, "main");
        assert_ne!(scenarios[0].relative_path, scenarios[1].relative_path);
        assert!(
            scenarios[0]
                .relative_path
                .contains("crate_a-1111111111111111"),
            "expected first synthetic path to retain its running binary context"
        );
        assert!(
            scenarios[1].relative_path.contains("api-2222222222222222"),
            "expected second synthetic path to retain its running binary context"
        );
    }

    #[test]
    fn host_enumeration_ignores_unscoped_doc_test_names_without_running_context() {
        let output = r#"
Running tests/api.rs (target/debug/deps/api-2222222222222222)
smoke: test
Doc-tests crate_a
main: test
Doc-tests crate_b
main: test
"#;

        let scenarios = parse_enumerated_host_tests(output);
        assert_eq!(scenarios.len(), 1);
        assert_eq!(scenarios[0].scenario_name, "smoke");
        assert_eq!(scenarios[0].suite_name, "enumerated");
        assert!(
            scenarios[0].relative_path.contains("api-2222222222222222"),
            "expected scoped host test to retain its running binary context"
        );
    }
}
