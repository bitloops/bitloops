use super::*;
use serde_json::Value;
use std::path::{Path, PathBuf};

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
    pub(super) artefacts: Vec<JsTsArtefact>,
    pub(super) edges: Vec<JsTsDependencyEdge>,
    pub(super) parsed_query: Option<ParsedDevqlQuery>,
    pub(super) query_sql: Option<String>,
    pub(super) query_error: Option<anyhow::Error>,
    pub(super) cfg: DevqlConfig,
    pub(super) log_workspace: Option<tempfile::TempDir>,
    pub(super) log_file_path: Option<PathBuf>,
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
    }

    pub(super) fn test_cfg() -> DevqlConfig {
        DevqlConfig {
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
        }
    }

    pub(super) fn init_test_logger(&mut self) {
        if self.log_workspace.is_some() {
            return;
        }
        let workspace = tempfile::tempdir().expect("create temp logger workspace");
        let log_file_path = workspace.path().join(".bitloops").join("logs").join("bitloops.log");
        self.log_file_path = Some(log_file_path);
        self.log_workspace = Some(workspace);
    }

    pub(super) fn logger_workspace_path(&self) -> &Path {
        self.log_workspace
            .as_ref()
            .expect("logger workspace should be initialized")
            .path()
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
                artefact.language_kind == language_kind
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
                    && edge.to_target_symbol_fqn.as_deref()
                        == expectation.to_target_symbol_fqn
                    && edge.to_symbol_ref.as_deref() == expectation.to_symbol_ref
                    && expectation.metadata_key.is_none_or(|key| {
                        edge.metadata
                            .get(key)
                            .and_then(|value| value.as_str())
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
}
