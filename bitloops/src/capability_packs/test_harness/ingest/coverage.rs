// Command handler for ingesting coverage data. Creates one coverage_captures row
// per invocation and N coverage_hits rows. No fan-out through test_links.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::capability_packs::test_harness::storage::TestHarnessCoverageGateway;
use crate::host::capability_host::gateways::RelationalGateway;
use crate::models::{
    CoverageCaptureRecord, CoverageDiagnosticRecord, CoverageFormat, CoverageHitRecord, ScopeKind,
};

#[derive(Debug, Clone)]
pub struct IngestCoverageSummary {
    pub format: CoverageFormat,
    pub scope_kind: ScopeKind,
    pub hits: usize,
    pub classifications: usize,
    pub diagnostics: usize,
}

#[derive(Debug, Clone)]
struct LcovFileCoverage {
    source_file: String,
    line_hits: HashMap<i64, i64>,
    branches: Vec<LcovBranchCoverage>,
}

#[derive(Debug, Clone, Copy)]
struct LcovBranchCoverage {
    line: i64,
    branch_id: i64,
    hit_count: i64,
}

#[allow(clippy::too_many_arguments)]
pub fn execute(
    store: &mut dyn TestHarnessCoverageGateway,
    relational: &dyn RelationalGateway,
    coverage_path: &Path,
    commit_sha: &str,
    scope_kind: ScopeKind,
    tool: &str,
    test_artefact_id: Option<&str>,
    format: CoverageFormat,
) -> Result<IngestCoverageSummary> {
    let repo_id = relational.load_repo_id_for_commit(commit_sha)?;

    let capture_id = format!(
        "capture:{commit_sha}:{}:{}",
        scope_kind,
        test_artefact_id.unwrap_or("all")
    );
    let captured_at = chrono_now();

    let has_branches = format == CoverageFormat::LlvmJson;

    let capture = CoverageCaptureRecord {
        capture_id: capture_id.clone(),
        repo_id: repo_id.clone(),
        commit_sha: commit_sha.to_string(),
        tool: tool.to_string(),
        format,
        scope_kind,
        subject_test_symbol_id: test_artefact_id.map(|s| s.to_string()),
        line_truth: true,
        branch_truth: has_branches,
        captured_at,
        status: "complete".to_string(),
        metadata_json: None,
    };

    let (hits, diagnostics) = match format {
        CoverageFormat::Lcov => {
            ingest_lcov(relational, coverage_path, commit_sha, &repo_id, &capture_id)?
        }
        CoverageFormat::LlvmJson => {
            crate::capability_packs::test_harness::ingest::parse_llvm_json::ingest_llvm_json(
                relational,
                coverage_path,
                commit_sha,
                &repo_id,
                &capture_id,
            )?
        }
    };

    store.insert_coverage_capture(&capture)?;
    store.insert_coverage_hits(&hits)?;
    store.insert_coverage_diagnostics(&diagnostics)?;

    let classifications = store.rebuild_classifications_from_coverage(commit_sha)?;

    Ok(IngestCoverageSummary {
        format,
        scope_kind,
        hits: hits.len(),
        classifications,
        diagnostics: diagnostics.len(),
    })
}

pub fn format_summary(commit_sha: &str, summary: &IngestCoverageSummary) -> String {
    format!(
        "ingested {} coverage for commit {} (scope: {}, hits: {}, classifications: {}, diagnostics: {})",
        summary.format,
        commit_sha,
        summary.scope_kind,
        summary.hits,
        summary.classifications,
        summary.diagnostics
    )
}

pub fn print_summary(commit_sha: &str, summary: &IngestCoverageSummary) {
    println!("{}", format_summary(commit_sha, summary));
}

fn ingest_lcov(
    relational: &dyn RelationalGateway,
    lcov_path: &Path,
    commit_sha: &str,
    repo_id: &str,
    capture_id: &str,
) -> Result<(Vec<CoverageHitRecord>, Vec<CoverageDiagnosticRecord>)> {
    let (report, parse_diagnostics) =
        parse_lcov_report(lcov_path, capture_id, repo_id, commit_sha)?;
    let mut hits = Vec::new();
    let mut diagnostics = parse_diagnostics;
    let mut diag_idx = diagnostics.len();

    for file in &report {
        let artefacts = relational.load_artefacts_for_file_lines(commit_sha, &file.source_file)?;
        if artefacts.is_empty() {
            diagnostics.push(CoverageDiagnosticRecord {
                diagnostic_id: format!("diag:{capture_id}:unmapped:{diag_idx}"),
                capture_id: capture_id.to_string(),
                repo_id: repo_id.to_string(),
                commit_sha: commit_sha.to_string(),
                path: Some(file.source_file.clone()),
                line: None,
                severity: "warning".to_string(),
                code: "unmapped_file".to_string(),
                message: format!(
                    "coverage file '{}' has no matching production artefacts",
                    file.source_file
                ),
                metadata_json: None,
            });
            diag_idx += 1;
            continue;
        }

        for (production_symbol_id, start_line, end_line) in &artefacts {
            for (&line_number, &hit_count) in &file.line_hits {
                if line_number < *start_line || line_number > *end_line {
                    continue;
                }
                hits.push(CoverageHitRecord {
                    capture_id: capture_id.to_string(),
                    production_symbol_id: production_symbol_id.clone(),
                    file_path: file.source_file.clone(),
                    line: line_number,
                    branch_id: -1,
                    covered: hit_count > 0,
                    hit_count,
                });
            }

            for branch in &file.branches {
                if branch.line < *start_line || branch.line > *end_line {
                    continue;
                }
                hits.push(CoverageHitRecord {
                    capture_id: capture_id.to_string(),
                    production_symbol_id: production_symbol_id.clone(),
                    file_path: file.source_file.clone(),
                    line: branch.line,
                    branch_id: branch.branch_id,
                    covered: branch.hit_count > 0,
                    hit_count: branch.hit_count,
                });
            }
        }
    }

    Ok((hits, diagnostics))
}

fn chrono_now() -> String {
    // Simple ISO-8601-ish timestamp without pulling in chrono crate
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", duration.as_secs())
}

fn parse_lcov_report(
    lcov_path: &Path,
    capture_id: &str,
    repo_id: &str,
    commit_sha: &str,
) -> Result<(Vec<LcovFileCoverage>, Vec<CoverageDiagnosticRecord>)> {
    let raw = fs::read_to_string(lcov_path)
        .with_context(|| format!("failed to read LCOV file {}", lcov_path.display()))?;

    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    let mut current: Option<LcovFileCoverage> = None;
    let mut diag_idx: usize = 0;

    let make_diag =
        |diag_idx: &mut usize, line_num: usize, source_file: Option<&str>, message: String| {
            let diag = CoverageDiagnosticRecord {
                diagnostic_id: format!("diag:{capture_id}:parse:{}", *diag_idx),
                capture_id: capture_id.to_string(),
                repo_id: repo_id.to_string(),
                commit_sha: commit_sha.to_string(),
                path: source_file.map(|s| s.to_string()),
                line: Some(line_num as i64),
                severity: "warning".to_string(),
                code: "malformed_line".to_string(),
                message,
                metadata_json: None,
            };
            *diag_idx += 1;
            diag
        };

    for (line_num, line) in raw.lines().enumerate() {
        let trimmed = line.trim();

        if let Some(path) = trimmed.strip_prefix("SF:") {
            if let Some(existing) = current.take() {
                files.push(existing);
            }
            current = Some(LcovFileCoverage {
                source_file: normalize_lcov_path(path),
                line_hits: HashMap::new(),
                branches: Vec::new(),
            });
            continue;
        }

        if trimmed == "end_of_record" {
            if let Some(existing) = current.take() {
                files.push(existing);
            }
            continue;
        }

        let Some(active) = current.as_mut() else {
            continue;
        };

        if let Some(da) = trimmed.strip_prefix("DA:") {
            let mut parts = da.splitn(3, ',');
            let (Some(line_no_raw), Some(hit_count_raw)) = (parts.next(), parts.next()) else {
                diagnostics.push(make_diag(
                    &mut diag_idx,
                    line_num + 1,
                    Some(&active.source_file),
                    format!("malformed DA line: '{trimmed}'"),
                ));
                continue;
            };
            let (Ok(line_no), Ok(hit_count)) =
                (line_no_raw.parse::<i64>(), hit_count_raw.parse::<i64>())
            else {
                diagnostics.push(make_diag(
                    &mut diag_idx,
                    line_num + 1,
                    Some(&active.source_file),
                    format!("unparseable DA values: '{trimmed}'"),
                ));
                continue;
            };
            active.line_hits.insert(line_no, hit_count);
            continue;
        }

        if let Some(brda) = trimmed.strip_prefix("BRDA:") {
            let parts: Vec<&str> = brda.split(',').collect();
            if parts.len() != 4 {
                diagnostics.push(make_diag(
                    &mut diag_idx,
                    line_num + 1,
                    Some(&active.source_file),
                    format!("malformed BRDA line (expected 4 fields): '{trimmed}'"),
                ));
                continue;
            }

            let (Ok(line_no), Ok(block_no), Ok(branch_no)) = (
                parts[0].parse::<i64>(),
                parts[1].parse::<i64>(),
                parts[2].parse::<i64>(),
            ) else {
                diagnostics.push(make_diag(
                    &mut diag_idx,
                    line_num + 1,
                    Some(&active.source_file),
                    format!("unparseable BRDA values: '{trimmed}'"),
                ));
                continue;
            };
            let hit_count = if parts[3] == "-" {
                0
            } else {
                parts[3].parse::<i64>().unwrap_or(0)
            };

            active.branches.push(LcovBranchCoverage {
                line: line_no,
                branch_id: block_no.saturating_mul(1000).saturating_add(branch_no),
                hit_count,
            });
        }
    }

    if let Some(existing) = current.take() {
        files.push(existing);
    }

    if files.is_empty() {
        anyhow::bail!(
            "no LCOV file records found in {} (expected at least one SF section)",
            lcov_path.display()
        );
    }

    Ok((files, diagnostics))
}

fn normalize_lcov_path(path: &str) -> String {
    path.trim().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use anyhow::Result;

    use super::{execute, format_summary, parse_lcov_report};
    use crate::capability_packs::test_harness::storage::TestHarnessCoverageGateway;
    use crate::host::capability_host::gateways::RelationalGateway;
    use crate::models::{
        CoverageCaptureRecord, CoverageDiagnosticRecord, CoverageFormat, CoverageHitRecord,
        ScopeKind,
    };

    #[derive(Default)]
    struct FakeCoverageStore {
        captures: Vec<CoverageCaptureRecord>,
        hits: Vec<CoverageHitRecord>,
        diagnostics: Vec<CoverageDiagnosticRecord>,
        rebuild_commits: Vec<String>,
        classifications: usize,
    }

    impl TestHarnessCoverageGateway for FakeCoverageStore {
        fn insert_coverage_capture(&mut self, capture: &CoverageCaptureRecord) -> Result<()> {
            self.captures.push(capture.clone());
            Ok(())
        }

        fn insert_coverage_hits(&mut self, hits: &[CoverageHitRecord]) -> Result<()> {
            self.hits.extend_from_slice(hits);
            Ok(())
        }

        fn insert_coverage_diagnostics(
            &mut self,
            diagnostics: &[CoverageDiagnosticRecord],
        ) -> Result<()> {
            self.diagnostics.extend_from_slice(diagnostics);
            Ok(())
        }

        fn rebuild_classifications_from_coverage(&mut self, commit_sha: &str) -> Result<usize> {
            self.rebuild_commits.push(commit_sha.to_string());
            Ok(self.classifications)
        }
    }

    #[derive(Default)]
    struct FakeRelationalGateway {
        repo_id: String,
        artefacts_by_file: HashMap<String, Vec<(String, i64, i64)>>,
    }

    impl RelationalGateway for FakeRelationalGateway {
        fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
            unreachable!("unused in coverage tests")
        }

        fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
            unreachable!("unused in coverage tests")
        }

        fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
            Ok(self.repo_id.clone())
        }

        fn load_current_production_artefacts(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<crate::models::ProductionArtefact>> {
            unreachable!("unused in coverage tests")
        }

        fn load_production_artefacts(
            &self,
            _commit_sha: &str,
        ) -> Result<Vec<crate::models::ProductionArtefact>> {
            unreachable!("unused in coverage tests")
        }

        fn load_artefacts_for_file_lines(
            &self,
            _commit_sha: &str,
            file_path: &str,
        ) -> Result<Vec<(String, i64, i64)>> {
            Ok(self
                .artefacts_by_file
                .get(file_path)
                .cloned()
                .unwrap_or_default())
        }
    }

    #[test]
    fn parse_lcov_report_collects_hits_branches_and_parse_diagnostics() {
        let temp = tempfile::NamedTempFile::new().expect("temp lcov");
        std::fs::write(
            temp.path(),
            "\
SF:C:\\repo\\src\\lib.rs
DA:10,3
DA:abc,2
DA:11,0
BRDA:10,0,1,2
BRDA:bad
end_of_record
",
        )
        .expect("write lcov");

        let (files, diagnostics) =
            parse_lcov_report(temp.path(), "capture:test", "repo:test", "commit-sha-123")
                .expect("parse lcov");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].source_file, "C:/repo/src/lib.rs");
        assert_eq!(files[0].line_hits.get(&10), Some(&3));
        assert_eq!(files[0].line_hits.get(&11), Some(&0));
        assert_eq!(files[0].branches.len(), 1);
        assert_eq!(files[0].branches[0].line, 10);
        assert_eq!(files[0].branches[0].branch_id, 1);
        assert_eq!(files[0].branches[0].hit_count, 2);
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics[0].message.contains("unparseable DA values"));
        assert!(diagnostics[1].message.contains("malformed BRDA line"));
    }

    #[test]
    fn execute_persists_lcov_capture_hits_and_unmapped_diagnostics() {
        let mut store = FakeCoverageStore {
            classifications: 7,
            ..Default::default()
        };
        let relational = FakeRelationalGateway {
            repo_id: "repo:test".to_string(),
            artefacts_by_file: HashMap::from([(
                "src/lib.rs".to_string(),
                vec![
                    ("prod:service".to_string(), 10, 11),
                    ("prod:branch".to_string(), 11, 11),
                ],
            )]),
        };
        let temp = tempfile::NamedTempFile::new().expect("temp lcov");
        std::fs::write(
            temp.path(),
            "\
SF:src/lib.rs
DA:10,3
DA:11,0
BRDA:11,0,1,2
end_of_record
SF:src/missing.rs
DA:5,1
end_of_record
",
        )
        .expect("write lcov");

        let summary = execute(
            &mut store,
            &relational,
            temp.path(),
            "commit-sha-123",
            ScopeKind::TestScenario,
            "cargo-llvm-cov",
            Some("test-symbol:login"),
            CoverageFormat::Lcov,
        )
        .expect("execute lcov ingest");

        assert_eq!(summary.hits, 5);
        assert_eq!(summary.classifications, 7);
        assert_eq!(summary.diagnostics, 1);
        assert_eq!(store.captures.len(), 1);
        let capture = &store.captures[0];
        assert_eq!(
            capture.capture_id,
            "capture:commit-sha-123:test_scenario:test-symbol:login"
        );
        assert_eq!(capture.repo_id, "repo:test");
        assert_eq!(capture.tool, "cargo-llvm-cov");
        assert_eq!(capture.format, CoverageFormat::Lcov);
        assert_eq!(capture.scope_kind, ScopeKind::TestScenario);
        assert_eq!(
            capture.subject_test_symbol_id.as_deref(),
            Some("test-symbol:login")
        );
        assert!(capture.line_truth);
        assert!(!capture.branch_truth);
        assert_eq!(capture.status, "complete");
        assert!(!capture.captured_at.is_empty());

        assert_eq!(store.hits.len(), 5);
        assert!(store.hits.iter().any(|hit| {
            hit.production_symbol_id == "prod:service"
                && hit.file_path == "src/lib.rs"
                && hit.line == 10
                && hit.branch_id == -1
                && hit.covered
                && hit.hit_count == 3
        }));
        assert!(store.hits.iter().any(|hit| {
            hit.production_symbol_id == "prod:service"
                && hit.line == 11
                && hit.branch_id == 1
                && hit.covered
                && hit.hit_count == 2
        }));
        assert!(store.hits.iter().any(|hit| {
            hit.production_symbol_id == "prod:branch"
                && hit.line == 11
                && hit.branch_id == -1
                && !hit.covered
                && hit.hit_count == 0
        }));
        assert_eq!(store.diagnostics.len(), 1);
        assert_eq!(store.diagnostics[0].code, "unmapped_file");
        assert_eq!(store.diagnostics[0].path.as_deref(), Some("src/missing.rs"));
        assert_eq!(store.rebuild_commits, vec!["commit-sha-123".to_string()]);

        let summary_text = format_summary("commit-sha-123", &summary);
        assert!(summary_text.contains("ingested lcov coverage for commit commit-sha-123"));
        assert!(summary_text.contains("hits: 5"));
        assert!(summary_text.contains("diagnostics: 1"));
    }
}
