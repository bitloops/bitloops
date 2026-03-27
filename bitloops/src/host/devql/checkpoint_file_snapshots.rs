#![allow(dead_code)]

use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CheckpointFileSnapshotActivityFilter<'a> {
    pub agent: Option<&'a str>,
    pub since: Option<&'a str>,
}

impl<'a> CheckpointFileSnapshotActivityFilter<'a> {
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
pub(crate) struct CheckpointFileSnapshotExistsSql<'a> {
    pub repo_id: &'a str,
    pub path_column: &'a str,
    pub blob_sha_column: &'a str,
    pub activity_filter: CheckpointFileSnapshotActivityFilter<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckpointFileSnapshotScope<'a> {
    Repository,
    Project(&'a str),
    File(&'a str),
}

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
pub(crate) struct CheckpointFileSnapshotDebugRow {
    pub path: String,
    pub blob_sha: String,
    pub checkpoint_count: usize,
    pub first_event_time: Option<String>,
    pub last_event_time: Option<String>,
}

pub(crate) struct CheckpointFileSnapshotGateway<'a> {
    relational: &'a RelationalStorage,
}

impl<'a> CheckpointFileSnapshotGateway<'a> {
    pub(crate) fn new(relational: &'a RelationalStorage) -> Self {
        Self { relational }
    }

    pub(crate) async fn list_matching_checkpoints(
        &self,
        repo_id: &str,
        path: &str,
        blob_sha: &str,
        activity_filter: CheckpointFileSnapshotActivityFilter<'_>,
        limit: usize,
    ) -> Result<Vec<CheckpointFileSnapshotMatch>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let sql = build_checkpoint_file_snapshot_lookup_sql(
            repo_id,
            path,
            blob_sha,
            activity_filter,
            limit,
        );
        let rows = self.relational.query_rows(&sql).await?;
        rows.into_iter().map(checkpoint_match_from_row).collect()
    }

    pub(crate) async fn list_debug_rows(
        &self,
        repo_id: &str,
        scope: CheckpointFileSnapshotScope<'_>,
        activity_filter: CheckpointFileSnapshotActivityFilter<'_>,
        limit: usize,
    ) -> Result<Vec<CheckpointFileSnapshotDebugRow>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let sql = build_checkpoint_file_snapshot_debug_sql(repo_id, scope, activity_filter, limit);
        let rows = self.relational.query_rows(&sql).await?;
        rows.into_iter()
            .map(checkpoint_debug_row_from_row)
            .collect()
    }
}

pub(crate) fn build_checkpoint_file_snapshot_exists_clause(
    params: CheckpointFileSnapshotExistsSql<'_>,
) -> String {
    let mut clauses = vec![
        format!("cfs.repo_id = '{}'", esc_pg(params.repo_id)),
        format!("cfs.path = {}", params.path_column),
        format!("cfs.blob_sha = {}", params.blob_sha_column),
    ];
    clauses.extend(params.activity_filter.sql_clauses("cfs"));

    format!(
        "EXISTS (SELECT 1 FROM checkpoint_file_snapshots cfs WHERE {})",
        clauses.join(" AND "),
    )
}

pub(crate) fn build_checkpoint_file_snapshot_lookup_sql(
    repo_id: &str,
    path: &str,
    blob_sha: &str,
    activity_filter: CheckpointFileSnapshotActivityFilter<'_>,
    limit: usize,
) -> String {
    let mut clauses = vec![
        format!("cfs.repo_id = '{}'", esc_pg(repo_id)),
        format!("cfs.blob_sha = '{}'", esc_pg(blob_sha)),
        format!(
            "({})",
            sql_path_candidates_clause("cfs.path", &build_path_candidates(path))
        ),
    ];
    clauses.extend(activity_filter.sql_clauses("cfs"));

    format!(
        "SELECT cfs.checkpoint_id, cfs.session_id, cfs.event_time, cfs.agent, \
                cfs.commit_sha, cfs.branch, cfs.strategy, cfs.path, cfs.blob_sha \
           FROM checkpoint_file_snapshots cfs \
          WHERE {} \
       ORDER BY cfs.event_time DESC, cfs.checkpoint_id DESC \
          LIMIT {}",
        clauses.join(" AND "),
        limit.max(1),
    )
}

pub(crate) fn build_checkpoint_file_snapshot_debug_sql(
    repo_id: &str,
    scope: CheckpointFileSnapshotScope<'_>,
    activity_filter: CheckpointFileSnapshotActivityFilter<'_>,
    limit: usize,
) -> String {
    let mut clauses = vec![format!("cfs.repo_id = '{}'", esc_pg(repo_id))];
    if let Some(scope_clause) = checkpoint_file_snapshot_scope_clause("cfs.path", scope) {
        clauses.push(scope_clause);
    }
    clauses.extend(activity_filter.sql_clauses("cfs"));

    format!(
        "SELECT cfs.path, cfs.blob_sha, COUNT(*) AS checkpoint_count, \
                MIN(cfs.event_time) AS first_event_time, MAX(cfs.event_time) AS last_event_time \
           FROM checkpoint_file_snapshots cfs \
          WHERE {} \
       GROUP BY cfs.path, cfs.blob_sha \
       ORDER BY last_event_time DESC, cfs.path, cfs.blob_sha \
          LIMIT {}",
        clauses.join(" AND "),
        limit.max(1),
    )
}

fn checkpoint_file_snapshot_scope_clause(
    column: &str,
    scope: CheckpointFileSnapshotScope<'_>,
) -> Option<String> {
    match scope {
        CheckpointFileSnapshotScope::Repository => None,
        CheckpointFileSnapshotScope::Project(project_path) => {
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
        CheckpointFileSnapshotScope::File(path) => Some(format!(
            "({})",
            sql_path_candidates_clause(column, &build_path_candidates(path))
        )),
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

fn checkpoint_debug_row_from_row(row: Value) -> Result<CheckpointFileSnapshotDebugRow> {
    Ok(CheckpointFileSnapshotDebugRow {
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
        .with_context(|| format!("missing `{field}` in checkpoint_file_snapshots row"))?;
    json_text_value(value)
        .with_context(|| format!("invalid `{field}` in checkpoint_file_snapshots row"))
}

fn json_optional_text_field(row: &Value, field: &str) -> Option<String> {
    row.get(field)
        .and_then(json_text_value)
        .filter(|value| !value.is_empty())
}

fn json_required_usize_field(row: &Value, field: &str) -> Result<usize> {
    let value = row
        .get(field)
        .with_context(|| format!("missing `{field}` in checkpoint_file_snapshots row"))?;

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
            .with_context(|| format!("parsing `{field}` from checkpoint_file_snapshots row"));
    }

    bail!("invalid `{field}` in checkpoint_file_snapshots row")
}

fn json_text_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const PROJECTION_TABLE_SQL: &str = "
        CREATE TABLE checkpoint_file_snapshots (
            repo_id TEXT NOT NULL,
            checkpoint_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            event_time TEXT NOT NULL,
            agent TEXT NOT NULL,
            branch TEXT NOT NULL,
            strategy TEXT NOT NULL,
            commit_sha TEXT NOT NULL,
            path TEXT NOT NULL,
            blob_sha TEXT NOT NULL
        );
    ";

    async fn sqlite_relational_with_projection(seed_sql: &str) -> RelationalStorage {
        let temp = tempdir().expect("temp dir");
        let db_path = temp.path().join("projection.sqlite");
        let sql = format!("{PROJECTION_TABLE_SQL}{seed_sql}");
        sqlite_exec_path_allow_create(&db_path, &sql)
            .await
            .expect("seed checkpoint file snapshots");
        std::mem::forget(temp);
        RelationalStorage::local_only(db_path)
    }

    #[test]
    fn build_exists_clause_matches_path_and_blob_with_activity_filter() {
        let sql = build_checkpoint_file_snapshot_exists_clause(CheckpointFileSnapshotExistsSql {
            repo_id: "repo-1",
            path_column: "a.path",
            blob_sha_column: "a.blob_sha",
            activity_filter: CheckpointFileSnapshotActivityFilter {
                agent: Some("codex"),
                since: Some("2026-03-20T00:00:00Z"),
            },
        });

        assert!(sql.contains("EXISTS (SELECT 1 FROM checkpoint_file_snapshots cfs WHERE"));
        assert!(sql.contains("cfs.repo_id = 'repo-1'"));
        assert!(sql.contains("cfs.path = a.path"));
        assert!(sql.contains("cfs.blob_sha = a.blob_sha"));
        assert!(sql.contains("cfs.agent = 'codex'"));
        assert!(sql.contains("cfs.event_time >= '2026-03-20T00:00:00Z'"));
        assert!(!sql.contains("blob_sha IN"));
    }

    #[tokio::test]
    async fn list_matching_checkpoints_uses_exact_snapshot_identity() {
        let relational = sqlite_relational_with_projection(
            "
            INSERT INTO checkpoint_file_snapshots VALUES
                ('repo-1', 'checkpoint-1', 'session-1', '2026-03-20T10:00:00Z', 'codex', 'main', 'manual', 'commit-1', 'src/lib.rs', 'blob-1'),
                ('repo-1', 'checkpoint-2', 'session-2', '2026-03-22T10:00:00Z', 'codex', 'main', 'manual', 'commit-2', 'src/lib.rs', 'blob-1'),
                ('repo-1', 'checkpoint-3', 'session-3', '2026-03-23T10:00:00Z', 'codex', 'main', 'manual', 'commit-3', 'src/other.rs', 'blob-1'),
                ('repo-1', 'checkpoint-4', 'session-4', '2026-03-24T10:00:00Z', 'codex', 'main', 'manual', 'commit-4', 'src/lib.rs', 'blob-2'),
                ('repo-2', 'checkpoint-5', 'session-5', '2026-03-25T10:00:00Z', 'codex', 'main', 'manual', 'commit-5', 'src/lib.rs', 'blob-1');
            ",
        )
        .await;
        let gateway = CheckpointFileSnapshotGateway::new(&relational);

        let rows = gateway
            .list_matching_checkpoints(
                "repo-1",
                "./src/lib.rs",
                "blob-1",
                CheckpointFileSnapshotActivityFilter::default(),
                10,
            )
            .await
            .expect("lookup matching checkpoints");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].checkpoint_id, "checkpoint-2");
        assert_eq!(rows[1].checkpoint_id, "checkpoint-1");
        assert!(rows.iter().all(|row| row.path == "src/lib.rs"));
        assert!(rows.iter().all(|row| row.blob_sha == "blob-1"));
    }

    #[tokio::test]
    async fn list_matching_checkpoints_returns_empty_when_projection_has_no_rows() {
        let relational = sqlite_relational_with_projection("").await;
        let gateway = CheckpointFileSnapshotGateway::new(&relational);

        let rows = gateway
            .list_matching_checkpoints(
                "repo-1",
                "src/lib.rs",
                "blob-1",
                CheckpointFileSnapshotActivityFilter::default(),
                10,
            )
            .await
            .expect("lookup empty projection");

        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn list_debug_rows_respects_repository_project_and_file_scopes() {
        let relational = sqlite_relational_with_projection(
            "
            INSERT INTO checkpoint_file_snapshots VALUES
                ('repo-1', 'checkpoint-1', 'session-1', '2026-03-20T10:00:00Z', 'codex', 'main', 'manual', 'commit-1', 'packages/api/src/caller.ts', 'blob-a'),
                ('repo-1', 'checkpoint-2', 'session-2', '2026-03-21T10:00:00Z', 'codex', 'main', 'manual', 'commit-2', 'packages/api/src/caller.ts', 'blob-a'),
                ('repo-1', 'checkpoint-3', 'session-3', '2026-03-22T10:00:00Z', 'codex', 'main', 'manual', 'commit-3', 'packages/api/src/target.ts', 'blob-b'),
                ('repo-1', 'checkpoint-4', 'session-4', '2026-03-23T10:00:00Z', 'copilot', 'main', 'manual', 'commit-4', 'packages/web/src/page.ts', 'blob-c'),
                ('repo-2', 'checkpoint-5', 'session-5', '2026-03-24T10:00:00Z', 'codex', 'main', 'manual', 'commit-5', 'packages/api/src/caller.ts', 'blob-z');
            ",
        )
        .await;
        let gateway = CheckpointFileSnapshotGateway::new(&relational);

        let repository_rows = gateway
            .list_debug_rows(
                "repo-1",
                CheckpointFileSnapshotScope::Repository,
                CheckpointFileSnapshotActivityFilter::default(),
                10,
            )
            .await
            .expect("repository-scoped debug rows");
        assert_eq!(repository_rows.len(), 3);
        assert_eq!(repository_rows[1].path, "packages/api/src/target.ts");

        let project_rows = gateway
            .list_debug_rows(
                "repo-1",
                CheckpointFileSnapshotScope::Project("./packages/api"),
                CheckpointFileSnapshotActivityFilter {
                    agent: Some("codex"),
                    since: None,
                },
                10,
            )
            .await
            .expect("project-scoped debug rows");
        assert_eq!(project_rows.len(), 2);
        assert_eq!(project_rows[1].path, "packages/api/src/caller.ts");
        assert_eq!(project_rows[1].checkpoint_count, 2);

        let file_rows = gateway
            .list_debug_rows(
                "repo-1",
                CheckpointFileSnapshotScope::File("./packages/api/src/caller.ts"),
                CheckpointFileSnapshotActivityFilter::default(),
                10,
            )
            .await
            .expect("file-scoped debug rows");
        assert_eq!(file_rows.len(), 1);
        assert_eq!(file_rows[0].path, "packages/api/src/caller.ts");
        assert_eq!(file_rows[0].checkpoint_count, 2);
    }
}
