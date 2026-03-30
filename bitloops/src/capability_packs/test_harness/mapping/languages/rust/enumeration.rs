#![allow(dead_code)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::capability_packs::test_harness::mapping::model::{
    EnumeratedTestScenario, EnumerationMode, EnumerationResult, ReferenceCandidate,
    ScenarioDiscoverySource,
};

pub(crate) fn enumerate_rust_tests(repo_dir: &Path) -> EnumerationResult {
    if !repo_dir.join("Cargo.toml").exists() {
        return EnumerationResult::default();
    }

    let host_output = run_cargo_test_list(repo_dir, false);
    let doc_output = run_cargo_test_list(repo_dir, true);

    let mut result = EnumerationResult::default();
    let mut full_success = true;

    match host_output {
        Ok(output) => {
            result
                .scenarios
                .extend(parse_enumerated_host_tests(&output));
        }
        Err(error) => {
            full_success = false;
            result.notes.push(format!(
                "host enumeration unavailable: {}",
                error.replace('\n', " ")
            ));
        }
    }

    match doc_output {
        Ok(output) => {
            result.scenarios.extend(parse_enumerated_doctests(&output));
        }
        Err(error) => {
            full_success = false;
            result.notes.push(format!(
                "doctest enumeration unavailable: {}",
                error.replace('\n', " ")
            ));
        }
    }

    result.mode = if result.notes.is_empty() && full_success {
        EnumerationMode::Full
    } else if !result.scenarios.is_empty() {
        EnumerationMode::Partial
    } else {
        EnumerationMode::Skipped
    };
    result
}

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

fn run_cargo_test_list(repo_dir: &Path, doctests: bool) -> Result<String, String> {
    let mut command = Command::new("cargo");
    command
        .current_dir(repo_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("test")
        .arg("--workspace");
    if doctests {
        command.arg("--doc");
    }
    command.arg("--").arg("--list");

    let mut child = command.spawn().map_err(|error| {
        format!(
            "failed to execute cargo test list in {}: {}",
            repo_dir.display(),
            error
        )
    })?;
    let timeout = Duration::from_secs(30);
    let deadline = Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|error| format!("failed waiting for cargo test list: {error}"))?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}\n{stderr}");
                return if output.status.success() {
                    Ok(combined)
                } else {
                    Err(combined)
                };
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let output = child.wait_with_output().ok();
                let combined = output
                    .map(|output| {
                        format!(
                            "{}\n{}",
                            String::from_utf8_lossy(&output.stdout),
                            String::from_utf8_lossy(&output.stderr)
                        )
                    })
                    .unwrap_or_default();
                return Err(format!(
                    "timed out after {}s while listing {}tests{}",
                    timeout.as_secs(),
                    if doctests { "doc " } else { "" },
                    if combined.trim().is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", combined.replace('\n', " "))
                    }
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(200)),
            Err(error) => return Err(format!("failed polling cargo test list: {error}")),
        }
    }
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
