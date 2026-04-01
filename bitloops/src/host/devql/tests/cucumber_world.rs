use super::*;
use crate::capability_packs::knowledge::{AssociateKnowledgeResult, IngestKnowledgeResult};
use crate::capability_packs::test_harness::mapping::model::DiscoveryIssue;
use crate::daemon::EnrichmentQueueStatus;
use crate::host::capability_host::CapabilityHealthResult;
use crate::host::devql::knowledge_support::KnowledgeBddHarness;
use crate::models::{TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[derive(Debug)]
pub(super) struct EdgeExpectation<'a> {
    pub(super) edge_kind: &'a str,
    pub(super) from_symbol_fqn: &'a str,
    pub(super) to_target_symbol_fqn: Option<&'a str>,
    pub(super) to_symbol_ref: Option<&'a str>,
    pub(super) metadata_key: Option<&'a str>,
    pub(super) metadata_value: Option<&'a str>,
}

#[derive(Debug, cucumber::World)]
pub(super) struct DevqlBddWorld {
    pub(super) scenario_id: Option<String>,
    pub(super) source_path: Option<String>,
    pub(super) source_language: Option<String>,
    pub(super) source_content: Option<String>,
    pub(super) rust_source_path: Option<String>,
    pub(super) rust_source_content: Option<String>,
    pub(super) artefacts: Vec<LanguageArtefact>,
    pub(super) edges: Vec<DependencyEdge>,
    pub(super) parsed_query: Option<ParsedDevqlQuery>,
    pub(super) query_sql: Option<String>,
    pub(super) query_error: Option<anyhow::Error>,
    pub(super) cfg: DevqlConfig,
    pub(super) log_workspace: Option<tempfile::TempDir>,
    pub(super) log_file_path: Option<PathBuf>,
    // Test harness state
    pub(super) production_sources: Vec<(String, String)>,
    pub(super) test_sources: Vec<(String, String)>,
    pub(super) discovered_suites: Vec<TestArtefactCurrentRecord>,
    pub(super) discovered_scenarios: Vec<TestArtefactCurrentRecord>,
    pub(super) materialized_links: Vec<TestArtefactEdgeCurrentRecord>,
    pub(super) discovery_issues: Vec<DiscoveryIssue>,
    pub(super) tests_query_response: Option<Value>,
    pub(super) scenario_workspace: Option<TempDir>,
    pub(super) health_results: HashMap<String, CapabilityHealthResult>,
    pub(super) enrichment_status: Option<EnrichmentQueueStatus>,
    pub(super) operation_error: Option<String>,
    pub(super) operation_output: Vec<String>,
    pub(super) knowledge: Option<KnowledgeBddHarness>,
    pub(super) knowledge_last_ingest: Option<IngestKnowledgeResult>,
    pub(super) knowledge_last_association: Option<AssociateKnowledgeResult>,
    pub(super) knowledge_last_error: Option<anyhow::Error>,
    pub(super) knowledge_ids: HashMap<String, String>,
}

impl Default for DevqlBddWorld {
    fn default() -> Self {
        Self {
            scenario_id: None,
            source_path: None,
            source_language: None,
            source_content: None,
            rust_source_path: None,
            rust_source_content: None,
            artefacts: Vec::new(),
            edges: Vec::new(),
            parsed_query: None,
            query_sql: None,
            query_error: None,
            cfg: Self::test_cfg(),
            log_workspace: None,
            log_file_path: None,
            production_sources: Vec::new(),
            test_sources: Vec::new(),
            discovered_suites: Vec::new(),
            discovered_scenarios: Vec::new(),
            materialized_links: Vec::new(),
            discovery_issues: Vec::new(),
            tests_query_response: None,
            scenario_workspace: None,
            health_results: HashMap::new(),
            enrichment_status: None,
            operation_error: None,
            operation_output: Vec::new(),
            knowledge: None,
            knowledge_last_ingest: None,
            knowledge_last_association: None,
            knowledge_last_error: None,
            knowledge_ids: HashMap::new(),
        }
    }
}

impl DevqlBddWorld {
    pub(super) fn reset(&mut self) {
        self.scenario_id = None;
        self.source_path = None;
        self.source_language = None;
        self.source_content = None;
        self.rust_source_path = None;
        self.rust_source_content = None;
        self.artefacts.clear();
        self.edges.clear();
        self.parsed_query = None;
        self.query_sql = None;
        self.query_error = None;
        self.cfg = Self::test_cfg();
        self.log_workspace = None;
        self.log_file_path = None;
        self.production_sources.clear();
        self.test_sources.clear();
        self.discovered_suites.clear();
        self.discovered_scenarios.clear();
        self.materialized_links.clear();
        self.discovery_issues.clear();
        self.tests_query_response = None;
        self.scenario_workspace = None;
        self.health_results.clear();
        self.enrichment_status = None;
        self.operation_error = None;
        self.operation_output.clear();
        self.knowledge = None;
        self.knowledge_last_ingest = None;
        self.knowledge_last_association = None;
        self.knowledge_last_error = None;
        self.knowledge_ids.clear();
    }

    pub(super) fn test_cfg() -> DevqlConfig {
        DevqlConfig {
            config_root: PathBuf::from("/tmp/repo"),
            repo_root: PathBuf::from("/tmp/repo"),
            repo: RepoIdentity {
                provider: "github".to_string(),
                organization: "bitloops".to_string(),
                name: "temp2".to_string(),
                identity: "github/bitloops/temp2".to_string(),
                repo_id: deterministic_uuid("repo://github/bitloops/temp2"),
            },
            pg_dsn: None,
            clickhouse_url: "http://localhost:8123".to_string(),
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: "default".to_string(),
            semantic_provider: None,
            semantic_model: None,
            semantic_api_key: None,
            semantic_base_url: None,
        }
    }

    pub(super) fn init_test_logger(&mut self) {
        if self.log_workspace.is_some() {
            return;
        }
        let workspace = tempfile::tempdir().expect("create temp logger workspace");
        let log_file_path = workspace
            .path()
            .join("state-root")
            .join("bitloops")
            .join("logs")
            .join("bitloops.log");
        self.log_file_path = Some(log_file_path);
        self.log_workspace = Some(workspace);
    }

    pub(super) fn logger_workspace_path(&self) -> &Path {
        self.log_workspace
            .as_ref()
            .expect("logger workspace should be initialized")
            .path()
    }

    pub(super) fn ensure_scenario_workspace(&mut self) -> &Path {
        if self.scenario_workspace.is_none() {
            self.scenario_workspace =
                Some(tempfile::tempdir().expect("create temp resilience workspace"));
        }
        self.scenario_workspace
            .as_ref()
            .expect("scenario workspace should be initialized")
            .path()
    }

    pub(super) fn scenario_repo_root(&mut self) -> PathBuf {
        let path = self.ensure_scenario_workspace().join("repo");
        std::fs::create_dir_all(&path).expect("create scenario repo root");
        path
    }

    pub(super) fn scenario_config_override_root(&mut self) -> PathBuf {
        let path = self.ensure_scenario_workspace().join("config-root");
        std::fs::create_dir_all(&path).expect("create scenario config root");
        path
    }

    pub(super) fn scenario_state_override_root(&mut self) -> PathBuf {
        let path = self.ensure_scenario_workspace().join("state-root");
        std::fs::create_dir_all(&path).expect("create scenario state root");
        path
    }

    pub(super) fn scenario_bin_dir(&mut self) -> PathBuf {
        let path = self.ensure_scenario_workspace().join("bin");
        std::fs::create_dir_all(&path).expect("create scenario bin dir");
        path
    }

    pub(super) fn read_log_entries(&self) -> Vec<Value> {
        let Some(log_file_path) = &self.log_file_path else {
            return Vec::new();
        };
        let Ok(content) = std::fs::read_to_string(log_file_path) else {
            return Vec::new();
        };
        content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<Value>(line)
                    .unwrap_or_else(|err| panic!("invalid log line `{line}`: {err}"))
            })
            .collect()
    }

    pub(super) fn assert_artefact(
        &self,
        language_kind: &str,
        canonical_kind: Option<&str>,
        name: &str,
    ) {
        assert!(
            self.artefacts.iter().any(|artefact| {
                artefact.language_kind.as_str() == language_kind
                    && artefact.canonical_kind.as_deref() == canonical_kind
                    && artefact.name == name
            }),
            "expected artefact {language_kind:?}/{canonical_kind:?}/{name:?}\nactual: {:#?}",
            self.artefacts
        );
    }

    pub(super) fn assert_edge(&self, expectation: EdgeExpectation<'_>) {
        assert!(
            self.edges.iter().any(|edge| {
                edge.edge_kind == expectation.edge_kind
                    && edge.from_symbol_fqn == expectation.from_symbol_fqn
                    && edge.to_target_symbol_fqn.as_deref() == expectation.to_target_symbol_fqn
                    && edge.to_symbol_ref.as_deref() == expectation.to_symbol_ref
                    && expectation.metadata_key.is_none_or(|key| {
                        edge.metadata.get(key).and_then(|value| value.as_str())
                            == expectation.metadata_value
                    })
            }),
            "expected edge {:#?}\nactual: {:#?}",
            expectation,
            self.edges
        );
    }

    pub(super) fn assert_sql_contains(&self, fragment: &str) {
        let sql = self
            .query_sql
            .as_deref()
            .expect("query SQL should be built before asserting");
        assert!(
            sql.contains(fragment),
            "expected SQL fragment `{fragment}` in:\n{sql}"
        );
    }

    pub(super) fn init_knowledge_harness(&mut self) {
        if self.knowledge.is_none() {
            self.knowledge = Some(
                KnowledgeBddHarness::new()
                    .unwrap_or_else(|err| panic!("initialize knowledge bdd harness: {err:#}")),
            );
        }
    }

    pub(super) fn knowledge_harness_mut(&mut self) -> &mut KnowledgeBddHarness {
        self.knowledge
            .as_mut()
            .expect("knowledge harness should be initialized")
    }

    pub(super) fn remember_id(&mut self, key: &str, value: impl Into<String>) {
        self.knowledge_ids.insert(key.to_string(), value.into());
    }

    pub(super) fn resolve_placeholders(&self, raw: &str) -> String {
        let mut out = String::with_capacity(raw.len());
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '<' {
                out.push(ch);
                continue;
            }

            let mut key = String::new();
            let mut found_end = false;
            for next in chars.by_ref() {
                if next == '>' {
                    found_end = true;
                    break;
                }
                key.push(next);
            }

            if !found_end {
                out.push('<');
                out.push_str(key.as_str());
                break;
            }

            if let Some(value) = self.knowledge_ids.get(key.as_str()) {
                out.push_str(value.as_str());
            } else {
                out.push('<');
                out.push_str(key.as_str());
                out.push('>');
            }
        }
        out
    }
}
