use std::collections::HashMap;

use crate::domain::{ArtefactRecord, TestLinkRecord};

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredTestSuite {
    pub(crate) name: String,
    pub(crate) start_line: i64,
    pub(crate) end_line: i64,
    pub(crate) scenarios: Vec<DiscoveredTestScenario>,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredTestScenario {
    pub(crate) name: String,
    pub(crate) start_line: i64,
    pub(crate) end_line: i64,
    pub(crate) reference_candidates: Vec<ReferenceCandidate>,
    pub(crate) discovery_source: ScenarioDiscoverySource,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredTestFile {
    pub(crate) relative_path: String,
    pub(crate) language: String,
    pub(crate) line_count: i64,
    pub(crate) reference_candidates: Vec<ReferenceCandidate>,
    pub(crate) suites: Vec<DiscoveredTestSuite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScenarioDiscoverySource {
    Source,
    MacroGenerated,
    Doctest,
    Enumeration,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ReferenceCandidate {
    SymbolName(String),
    ScopedSymbol(String),
    SourcePath(String),
    ExplicitTarget { path: String, start_line: i64 },
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveryIssue {
    pub(crate) path: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TestDiscoveryBatch {
    pub(crate) files: Vec<DiscoveredTestFile>,
    pub(crate) issues: Vec<DiscoveryIssue>,
}

#[derive(Debug, Clone)]
pub(crate) struct EnumeratedTestScenario {
    pub(crate) language: String,
    pub(crate) suite_name: String,
    pub(crate) scenario_name: String,
    pub(crate) relative_path: String,
    pub(crate) start_line: i64,
    pub(crate) reference_candidates: Vec<ReferenceCandidate>,
    pub(crate) discovery_source: ScenarioDiscoverySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnumerationMode {
    Skipped,
    Partial,
    Full,
}

#[derive(Debug, Clone)]
pub(crate) struct EnumerationResult {
    pub(crate) mode: EnumerationMode,
    pub(crate) scenarios: Vec<EnumeratedTestScenario>,
    pub(crate) notes: Vec<String>,
}

impl Default for EnumerationResult {
    fn default() -> Self {
        Self {
            mode: EnumerationMode::Skipped,
            scenarios: Vec::new(),
            notes: Vec::new(),
        }
    }
}

impl EnumerationResult {
    pub(crate) fn status_label(&self) -> &'static str {
        match self.mode {
            EnumerationMode::Skipped => "source-only",
            EnumerationMode::Partial => "hybrid-partial",
            EnumerationMode::Full => "hybrid-full",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ReconciledDiscovery {
    pub(crate) enumerated_scenarios: Vec<EnumeratedTestScenario>,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidateTestFile {
    pub(crate) relative_path: String,
    pub(crate) provider_index: usize,
    pub(crate) priority: u8,
}

#[derive(Debug, Default)]
pub(crate) struct ProductionIndex {
    pub(crate) by_simple_symbol: HashMap<String, Vec<usize>>,
    pub(crate) by_explicit_target: HashMap<(String, i64), usize>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StructuralMappingStats {
    pub(crate) files: usize,
    pub(crate) suites: usize,
    pub(crate) scenarios: usize,
    pub(crate) links: usize,
    pub(crate) enumerated_scenarios: usize,
}

#[derive(Debug)]
pub(crate) struct StructuralMappingOutput {
    pub(crate) artefacts: Vec<ArtefactRecord>,
    pub(crate) links: Vec<TestLinkRecord>,
    pub(crate) stats: StructuralMappingStats,
    pub(crate) enumeration_status: String,
    pub(crate) enumeration_notes: Vec<String>,
    pub(crate) issues: Vec<DiscoveryIssue>,
}
