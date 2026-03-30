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

    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.ends_with(": test") || trimmed.contains(" - ") {
            continue;
        }

        let name = trimmed.trim_end_matches(": test").trim();
        if name.is_empty()
            || name.starts_with("Doc-tests ")
            || name.starts_with("Running ")
            || name.ends_with(" benchmarks")
        {
            continue;
        }

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
            relative_path: "__synthetic_tests__/workspace.rs".to_string(),
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
