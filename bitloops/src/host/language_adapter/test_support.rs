use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::models::{TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord};

use super::LanguageAdapterContext;

#[derive(Debug, Clone)]
pub struct DiscoveredTestSuite {
    pub name: String,
    pub start_line: i64,
    pub end_line: i64,
    pub scenarios: Vec<DiscoveredTestScenario>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredTestScenario {
    pub name: String,
    pub start_line: i64,
    pub end_line: i64,
    pub reference_candidates: Vec<ReferenceCandidate>,
    pub discovery_source: ScenarioDiscoverySource,
}

#[derive(Debug, Clone)]
pub struct DiscoveredTestFile {
    pub relative_path: String,
    pub language: String,
    pub reference_candidates: Vec<ReferenceCandidate>,
    pub suites: Vec<DiscoveredTestSuite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioDiscoverySource {
    Source,
    MacroGenerated,
    Doctest,
    Enumeration,
}

impl ScenarioDiscoverySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScenarioDiscoverySource::Source => "source",
            ScenarioDiscoverySource::MacroGenerated => "macro_generated",
            ScenarioDiscoverySource::Doctest => "doctest",
            ScenarioDiscoverySource::Enumeration => "enumeration",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReferenceCandidate {
    SymbolName(String),
    ScopedSymbol(String),
    SourcePath(String),
    ExplicitTarget { path: String, start_line: i64 },
}

#[derive(Debug, Clone)]
pub struct DiscoveryIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct TestDiscoveryBatch {
    pub files: Vec<DiscoveredTestFile>,
    pub issues: Vec<DiscoveryIssue>,
}

#[derive(Debug, Clone)]
pub struct EnumeratedTestScenario {
    pub language: String,
    pub suite_name: String,
    pub scenario_name: String,
    pub relative_path: String,
    pub start_line: i64,
    pub reference_candidates: Vec<ReferenceCandidate>,
    pub discovery_source: ScenarioDiscoverySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumerationMode {
    Skipped,
    Partial,
    Full,
}

#[derive(Debug, Clone)]
pub struct EnumerationResult {
    pub mode: EnumerationMode,
    pub scenarios: Vec<EnumeratedTestScenario>,
    pub notes: Vec<String>,
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
    pub fn status_label(&self) -> &'static str {
        match self.mode {
            EnumerationMode::Skipped => "source-only",
            EnumerationMode::Partial => "hybrid-partial",
            EnumerationMode::Full => "hybrid-full",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReconciledDiscovery {
    pub enumerated_scenarios: Vec<EnumeratedTestScenario>,
}

#[derive(Debug, Clone)]
pub struct CandidateTestFile {
    pub relative_path: String,
    pub language_id: String,
    pub priority: u8,
}

#[derive(Debug, Default)]
pub struct ProductionIndex {
    pub by_simple_symbol: HashMap<String, Vec<usize>>,
    pub by_explicit_target: HashMap<(String, i64), usize>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StructuralMappingStats {
    pub files: usize,
    pub test_artefacts: usize,
    pub test_edges: usize,
    pub enumerated_scenarios: usize,
}

#[derive(Debug)]
pub struct StructuralMappingOutput {
    pub test_artefacts: Vec<TestArtefactCurrentRecord>,
    pub test_edges: Vec<TestArtefactEdgeCurrentRecord>,
    pub stats: StructuralMappingStats,
    pub enumeration_status: String,
    pub enumeration_notes: Vec<String>,
    pub issues: Vec<DiscoveryIssue>,
}

pub trait LanguageTestSupport: Send + Sync {
    fn language_id(&self) -> &'static str;

    fn priority(&self) -> u8;

    fn supports_path(&self, absolute_path: &Path, relative_path: &str) -> bool;

    fn discover_tests(
        &self,
        absolute_path: &Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile>;

    fn enumerate_tests(&self, _ctx: &LanguageAdapterContext) -> EnumerationResult {
        EnumerationResult::default()
    }

    fn reconcile(
        &self,
        _source_files: &[DiscoveredTestFile],
        enumeration: EnumerationResult,
    ) -> ReconciledDiscovery {
        ReconciledDiscovery {
            enumerated_scenarios: enumeration.scenarios,
        }
    }
}
