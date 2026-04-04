use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckpointFileSnapshotMatch {
    pub checkpoint_id: String,
    pub session_id: String,
    pub event_time: String,
    pub agent: String,
    pub commit_sha: String,
    pub branch: String,
    pub strategy: String,
    pub path: String,
    pub blob_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckpointFileDebugRow {
    pub path: String,
    pub blob_sha: String,
    pub checkpoint_count: usize,
    pub first_event_time: Option<String>,
    pub last_event_time: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CheckpointFileActivityFilter<'a> {
    pub agent: Option<&'a str>,
    pub since: Option<&'a str>,
}

impl<'a> CheckpointFileActivityFilter<'a> {
    fn sql_clauses(self, alias: &str) -> Vec<String> {
        let mut clauses = Vec::new();
        if let Some(agent) = self.agent.map(str::trim).filter(|value| !value.is_empty()) {
            clauses.push(format!("{alias}.agent = '{}'", esc_pg(agent)));
        }
        if let Some(since) = self.since.map(str::trim).filter(|value| !value.is_empty()) {
            clauses.push(format!("{alias}.event_time >= '{}'", esc_pg(since)));
        }
        clauses
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CheckpointFileExistsSql<'a> {
    pub repo_id: &'a str,
    pub path_column: &'a str,
    pub blob_sha_column: &'a str,
    pub activity_filter: CheckpointFileActivityFilter<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckpointFileScope<'a> {
    Repository,
    Project(&'a str),
    File(&'a str),
}

pub(crate) fn checkpoint_display_path(
    path_before: Option<&str>,
    path_after: Option<&str>,
) -> String {
    path_after
        .or(path_before)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_default()
}

pub(crate) fn build_checkpoint_file_exists_clause(params: CheckpointFileExistsSql<'_>) -> String {
    let mut clauses = vec![
        format!("cf.repo_id = '{}'", esc_pg(params.repo_id)),
        format!("cf.path_after = {}", params.path_column),
        format!("cf.blob_sha_after = {}", params.blob_sha_column),
    ];
    clauses.extend(params.activity_filter.sql_clauses("cf"));
    format!(
        "EXISTS (SELECT 1 FROM checkpoint_files cf WHERE {})",
        clauses.join(" AND "),
    )
}

pub(crate) fn build_checkpoint_file_lookup_sql(
    repo_id: &str,
    path: &str,
    blob_sha: &str,
    activity_filter: CheckpointFileActivityFilter<'_>,
    limit: usize,
) -> String {
    let mut clauses = vec![
        format!("cf.repo_id = '{}'", esc_pg(repo_id)),
        format!("cf.blob_sha_after = '{}'", esc_pg(blob_sha)),
        format!(
            "({})",
            sql_path_candidates_clause("cf.path_after", &build_path_candidates(path))
        ),
    ];
    clauses.extend(activity_filter.sql_clauses("cf"));
    format!(
        "SELECT cf.checkpoint_id, cf.session_id, cf.event_time, cf.agent, cf.commit_sha, \
                cf.branch, cf.strategy, cf.path_after AS path, cf.blob_sha_after AS blob_sha \
           FROM checkpoint_files cf \
          WHERE {} \
       ORDER BY cf.event_time DESC, cf.checkpoint_id DESC \
          LIMIT {}",
        clauses.join(" AND "),
        limit.max(1),
    )
}

pub(crate) fn build_checkpoint_file_debug_sql(
    repo_id: &str,
    scope: CheckpointFileScope<'_>,
    activity_filter: CheckpointFileActivityFilter<'_>,
    limit: usize,
) -> String {
    let mut clauses = vec![
        format!("cf.repo_id = '{}'", esc_pg(repo_id)),
        "cf.path_after IS NOT NULL".to_string(),
        "cf.blob_sha_after IS NOT NULL".to_string(),
    ];
    if let Some(scope_clause) = checkpoint_file_scope_clause("cf.path_after", scope) {
        clauses.push(scope_clause);
    }
    clauses.extend(activity_filter.sql_clauses("cf"));
    format!(
        "SELECT cf.path_after AS path, cf.blob_sha_after AS blob_sha, COUNT(*) AS checkpoint_count, \
                MIN(cf.event_time) AS first_event_time, MAX(cf.event_time) AS last_event_time \
           FROM checkpoint_files cf \
          WHERE {} \
       GROUP BY cf.path_after, cf.blob_sha_after \
       ORDER BY last_event_time DESC, cf.path_after, cf.blob_sha_after \
          LIMIT {}",
        clauses.join(" AND "),
        limit.max(1),
    )
}

fn checkpoint_file_scope_clause(column: &str, scope: CheckpointFileScope<'_>) -> Option<String> {
    match scope {
        CheckpointFileScope::Repository => None,
        CheckpointFileScope::Project(project_path) => {
            let candidates = build_path_candidates(project_path);
            if candidates.is_empty() {
                return Some("1 = 0".to_string());
            }
            let mut clauses = candidates
                .iter()
                .flat_map(|candidate| {
                    let prefix = format!("{}/%", escape_like_pattern(candidate));
                    [
                        format!("{column} = '{}'", esc_pg(candidate)),
                        sql_like_with_escape(column, &prefix),
                    ]
                })
                .collect::<Vec<_>>();
            clauses.sort();
            clauses.dedup();
            Some(format!("({})", clauses.join(" OR ")))
        }
        CheckpointFileScope::File(path) => Some(format!(
            "({})",
            sql_path_candidates_clause(column, &build_path_candidates(path))
        )),
    }
}

pub(crate) struct CheckpointFileGateway<'a> {
    relational: &'a RelationalStorage,
}

impl<'a> CheckpointFileGateway<'a> {
    pub(crate) fn new(relational: &'a RelationalStorage) -> Self {
        Self { relational }
    }

    pub(crate) async fn list_matching_checkpoints(
        &self,
        repo_id: &str,
        path: &str,
        blob_sha: &str,
        activity_filter: CheckpointFileActivityFilter<'_>,
        limit: usize,
    ) -> Result<Vec<CheckpointFileSnapshotMatch>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let sql = build_checkpoint_file_lookup_sql(repo_id, path, blob_sha, activity_filter, limit);
        let rows = self.relational.query_rows(&sql).await?;
        rows.into_iter()
            .map(checkpoint_match_from_row)
            .collect::<Result<Vec<_>>>()
    }

    pub(crate) async fn list_debug_rows(
        &self,
        repo_id: &str,
        scope: CheckpointFileScope<'_>,
        activity_filter: CheckpointFileActivityFilter<'_>,
        limit: usize,
    ) -> Result<Vec<CheckpointFileDebugRow>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let sql = build_checkpoint_file_debug_sql(repo_id, scope, activity_filter, limit);
        let rows = self.relational.query_rows(&sql).await?;
        rows.into_iter()
            .map(checkpoint_debug_row_from_row)
            .collect()
    }
}

fn checkpoint_match_from_row(row: Value) -> Result<CheckpointFileSnapshotMatch> {
    Ok(CheckpointFileSnapshotMatch {
        checkpoint_id: json_required_text_field(&row, "checkpoint_id")?,
        session_id: json_required_text_field(&row, "session_id")?,
        event_time: json_required_text_field(&row, "event_time")?,
        agent: json_required_text_field(&row, "agent")?,
        commit_sha: json_required_text_field(&row, "commit_sha")?,
        branch: json_required_text_field(&row, "branch")?,
        strategy: json_required_text_field(&row, "strategy")?,
        path: json_required_text_field(&row, "path")?,
        blob_sha: json_required_text_field(&row, "blob_sha")?,
    })
}

fn checkpoint_debug_row_from_row(row: Value) -> Result<CheckpointFileDebugRow> {
    Ok(CheckpointFileDebugRow {
        path: json_required_text_field(&row, "path")?,
        blob_sha: json_required_text_field(&row, "blob_sha")?,
        checkpoint_count: json_required_usize_field(&row, "checkpoint_count")?,
        first_event_time: json_optional_text_field(&row, "first_event_time"),
        last_event_time: json_optional_text_field(&row, "last_event_time"),
    })
}

fn json_required_text_field(row: &Value, field: &str) -> Result<String> {
    let value = row
        .get(field)
        .with_context(|| format!("missing `{field}` in checkpoint provenance row"))?;
    json_text_value(value)
        .with_context(|| format!("invalid `{field}` in checkpoint provenance row"))
}

fn json_optional_text_field(row: &Value, field: &str) -> Option<String> {
    row.get(field)
        .and_then(json_text_value)
        .filter(|value| !value.is_empty())
}

fn json_required_usize_field(row: &Value, field: &str) -> Result<usize> {
    let value = row
        .get(field)
        .with_context(|| format!("missing `{field}` in checkpoint provenance row"))?;
    if let Some(value) = value.as_u64() {
        return usize::try_from(value).with_context(|| format!("`{field}` does not fit in usize"));
    }
    if let Some(value) = value.as_i64() {
        return usize::try_from(value).with_context(|| format!("`{field}` does not fit in usize"));
    }
    if let Some(value) = value.as_str() {
        return value
            .trim()
            .parse::<usize>()
            .with_context(|| format!("parsing `{field}` from checkpoint provenance row"));
    }
    bail!("invalid `{field}` in checkpoint provenance row")
}

fn json_text_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}
