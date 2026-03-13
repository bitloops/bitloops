// Domain objects shared across command handlers and persistence boundaries.

#[derive(Debug, Clone)]
pub struct ArtefactRecord {
    pub artefact_id: String,
    pub repo_id: String,
    pub commit_sha: String,
    pub path: String,
    pub language: String,
    pub canonical_kind: String,
    pub language_kind: Option<String>,
    pub symbol_fqn: Option<String>,
    pub parent_artefact_id: Option<String>,
    pub start_line: i64,
    pub end_line: i64,
    pub signature: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProductionArtefact {
    pub artefact_id: String,
    pub symbol_fqn: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct QueriedArtefactRecord {
    pub artefact_id: String,
    pub symbol_fqn: Option<String>,
    pub canonical_kind: String,
    pub path: String,
    pub start_line: i64,
    pub end_line: i64,
}

#[derive(Debug, Clone)]
pub struct CoveringTestRecord {
    pub test_id: String,
    pub test_symbol_fqn: Option<String>,
    pub test_signature: Option<String>,
    pub test_path: String,
    pub suite_name: Option<String>,
    pub classification: Option<String>,
    pub classification_source: Option<String>,
    pub fan_out: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CoveragePairStats {
    pub total_rows: i64,
    pub covered_rows: i64,
}

#[derive(Debug, Clone)]
pub struct LatestTestRunRecord {
    pub status: String,
    pub duration_ms: Option<i64>,
    pub commit_sha: String,
}

#[derive(Debug, Clone)]
pub struct CoverageBranchRecord {
    pub line: i64,
    pub branch_id: i64,
    pub covered: bool,
    pub covering_test_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CoverageSummaryRecord {
    pub line_total: usize,
    pub line_covered: usize,
    pub branch_total: usize,
    pub branch_covered: usize,
    pub branches: Vec<CoverageBranchRecord>,
}

#[derive(Debug, Clone)]
pub struct ListedArtefactRecord {
    pub artefact_id: String,
    pub symbol_fqn: Option<String>,
    pub kind: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
}

#[derive(Debug, Clone)]
pub struct TestLinkRecord {
    pub test_link_id: String,
    pub test_artefact_id: String,
    pub production_artefact_id: String,
    pub commit_sha: String,
}

#[derive(Debug, Clone)]
pub struct TestScenarioRecord {
    pub artefact_id: String,
    pub path: String,
    pub suite_name: String,
    pub test_name: String,
}

#[derive(Debug, Clone)]
pub struct TestRunRecord {
    pub run_id: String,
    pub repo_id: String,
    pub commit_sha: String,
    pub test_artefact_id: String,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub ran_at: String,
}

#[derive(Debug, Clone)]
pub struct TestCoverageRecord {
    pub coverage_id: String,
    pub repo_id: String,
    pub commit_sha: String,
    pub test_artefact_id: String,
    pub artefact_id: String,
    pub line: i64,
    pub branch_id: Option<i64>,
    pub covered: bool,
    pub hit_count: i64,
}

#[derive(Debug, Clone)]
pub struct CoverageTarget {
    pub artefact_id: String,
    pub repo_id: String,
    pub start_line: i64,
    pub end_line: i64,
}

pub fn derive_test_classification(fan_out: i64, boundary_crossings: i64) -> &'static str {
    if fan_out >= 11 && boundary_crossings >= 3 {
        return "e2e";
    }
    if (4..=10).contains(&fan_out) && (1..=3).contains(&boundary_crossings) {
        return "integration";
    }
    if (1..=3).contains(&fan_out) && boundary_crossings <= 1 {
        return "unit";
    }

    if fan_out >= 11 || boundary_crossings >= 3 {
        "e2e"
    } else if fan_out >= 4 || boundary_crossings >= 2 {
        "integration"
    } else {
        "unit"
    }
}

#[cfg(test)]
mod tests {
    use super::derive_test_classification;

    #[test]
    fn classifies_unit_for_low_fan_out_and_boundary_crossings() {
        assert_eq!(derive_test_classification(1, 0), "unit");
        assert_eq!(derive_test_classification(3, 1), "unit");
    }

    #[test]
    fn classifies_integration_for_mid_fan_out() {
        assert_eq!(derive_test_classification(4, 1), "integration");
        assert_eq!(derive_test_classification(10, 3), "integration");
    }

    #[test]
    fn classifies_e2e_for_high_fan_out() {
        assert_eq!(derive_test_classification(11, 3), "e2e");
        assert_eq!(derive_test_classification(20, 4), "e2e");
    }

    #[test]
    fn falls_back_to_integration_when_boundary_crossings_are_high() {
        assert_eq!(derive_test_classification(2, 2), "integration");
    }
}
