use super::row::escape_like_literal;
use crate::host::devql::{esc_ch, esc_pg};
use std::collections::{HashMap, HashSet};

use super::super::super::GRAPHQL_GIT_SCAN_LIMIT;

#[allow(dead_code)]
fn build_clickhouse_chat_history_sql(
    repo_id: &str,
    path_candidates: &HashMap<String, Vec<String>>,
) -> String {
    let path_clause = path_candidates
        .values()
        .flat_map(|candidates| candidates.iter())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|candidate| format!("has(files_touched, '{}')", esc_ch(&candidate)))
        .collect::<Vec<_>>()
        .join(" OR ");

    format!(
        "SELECT event_time, checkpoint_id, session_id, agent, commit_sha, branch, strategy, \
                files_touched, payload \
           FROM checkpoint_events \
          WHERE repo_id = '{repo_id}' \
            AND event_type = 'checkpoint_committed' \
            AND checkpoint_id != '' \
            AND session_id != '' \
            AND ({path_clause}) \
       ORDER BY event_time DESC, checkpoint_id DESC \
          LIMIT {limit} FORMAT JSON",
        repo_id = esc_ch(repo_id),
        limit = GRAPHQL_GIT_SCAN_LIMIT,
    )
}

#[allow(dead_code)]
fn build_duckdb_chat_history_sql(
    repo_id: &str,
    path_candidates: &HashMap<String, Vec<String>>,
) -> String {
    let path_clause = path_candidates
        .values()
        .flat_map(|candidates| candidates.iter())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|candidate| {
            format!(
                "files_touched LIKE '%\"{}\"%' ESCAPE '\\'",
                esc_pg(&escape_like_literal(&candidate))
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");

    format!(
        "SELECT event_time, checkpoint_id, session_id, agent, commit_sha, branch, strategy, \
                files_touched, payload \
           FROM checkpoint_events \
          WHERE repo_id = '{repo_id}' \
            AND event_type = 'checkpoint_committed' \
            AND checkpoint_id <> '' \
            AND session_id <> '' \
            AND ({path_clause}) \
       ORDER BY event_time DESC, checkpoint_id DESC \
          LIMIT {limit}",
        repo_id = esc_pg(repo_id),
        limit = GRAPHQL_GIT_SCAN_LIMIT,
    )
}
