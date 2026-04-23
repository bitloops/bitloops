use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

pub(super) const ANALYTICS_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const ANALYTICS_MAX_ROWS: usize = 5_000;
pub(super) const ANALYTICS_FETCH_ROWS: usize = ANALYTICS_MAX_ROWS + 1;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsSqlColumn {
    pub(crate) name: String,
    pub(crate) logical_type: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsSqlResult {
    pub(crate) columns: Vec<AnalyticsSqlColumn>,
    pub(crate) rows: Value,
    pub(crate) row_count: usize,
    pub(crate) truncated: bool,
    pub(crate) duration_ms: u64,
    pub(crate) repo_ids: Vec<String>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnalyticsRepoScope {
    CurrentRepo,
    Explicit(Vec<String>),
    AllKnown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AnalyticsRepository {
    pub(super) repo_id: String,
    pub(super) repo_root: Option<PathBuf>,
    pub(super) provider: String,
    pub(super) organization: String,
    pub(super) name: String,
    pub(super) identity: String,
    pub(super) default_branch: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct RepoWatermark {
    pub(super) relational: String,
    pub(super) events: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct AnalyticsSourceTables {
    pub(super) repositories: Vec<Value>,
    pub(super) repo_sync_state: Vec<Value>,
    pub(super) current_file_state: Vec<Value>,
    pub(super) interaction_sessions: Vec<Value>,
    pub(super) interaction_turns: Vec<Value>,
    pub(super) interaction_events: Vec<Value>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct AnalyticsDerivedTables {
    pub(super) interaction_tool_invocations: Vec<Value>,
    pub(super) interaction_subagent_runs: Vec<Value>,
}

#[derive(Debug, Clone)]
pub(super) struct AnalyticsQueryResult {
    pub(super) columns: Vec<AnalyticsSqlColumn>,
    pub(super) rows: Vec<Value>,
    pub(super) truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ColumnKind {
    Text,
    Integer,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ColumnSpec {
    pub(super) name: &'static str,
    pub(super) kind: ColumnKind,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TableSpec {
    pub(super) name: &'static str,
    pub(super) columns: &'static [ColumnSpec],
    pub(super) key_columns: &'static [&'static str],
}
