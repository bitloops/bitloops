use anyhow::{Context, Result};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

use crate::capability_packs::semantic_clones::vector_backend::clear_sqlite_current_rows_for_paths;
use crate::config::{
    RepoPolicyScopeExclusions, discover_repo_policy_optional, resolve_repo_policy_scope_exclusions,
};

use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ScopeExclusionSnapshot {
    pub fingerprint: String,
    pub matcher: RepoExclusionMatcher,
}

pub(crate) async fn scope_exclusion_reconcile_needed(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
) -> Result<Option<String>> {
    if !sync::state::repo_sync_state_exists(relational, &cfg.repo.repo_id).await? {
        return Ok(None);
    }
    let snapshot = load_scope_exclusion_snapshot(&cfg.repo_root)?;
    let stored =
        sync::state::read_scope_exclusions_fingerprint(relational, &cfg.repo.repo_id).await?;
    if stored.is_none()
        && sync::state::read_last_sync_status(relational, &cfg.repo.repo_id)
            .await?
            .as_deref()
            == Some("running")
    {
        return Ok(None);
    }
    if stored.as_deref() == Some(snapshot.fingerprint.as_str()) {
        Ok(None)
    } else {
        Ok(Some(snapshot.fingerprint))
    }
}

pub(crate) async fn purge_scope_excluded_repo_data(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
) -> Result<String> {
    let snapshot = load_scope_exclusion_snapshot(&cfg.repo_root)?;
    let excluded_paths = load_excluded_paths(relational, &cfg.repo.repo_id, &snapshot.matcher)
        .await
        .context("loading stored repo paths for exclusion reconciliation")?;
    if excluded_paths.is_empty() {
        return Ok(snapshot.fingerprint);
    }

    let artefact_ids = load_distinct_string_set(
        relational,
        &excluded_artefact_ids_sql(&cfg.repo.repo_id, &excluded_paths),
        "artefact_id",
    )
    .await?;
    let blob_shas = load_distinct_string_set(
        relational,
        &excluded_blob_shas_sql(&cfg.repo.repo_id, &excluded_paths),
        "blob_sha",
    )
    .await?;
    let content_ids = load_distinct_string_set(
        relational,
        &excluded_content_ids_sql(&cfg.repo.repo_id, &excluded_paths),
        "content_id",
    )
    .await?;
    let statements = build_purge_statements(
        &cfg.repo.repo_id,
        &excluded_paths,
        &artefact_ids,
        &blob_shas,
        &content_ids,
    );
    if statements.is_empty() {
        return Ok(snapshot.fingerprint);
    }

    relational
        .exec_batch_transactional(&statements)
        .await
        .context("purging excluded DevQL rows from local relational storage")?;
    if relational.remote_client().is_some() {
        relational
            .exec_remote_batch_transactional(&statements)
            .await
            .context("purging excluded DevQL rows from remote relational storage")?;
    }
    let excluded_paths_vec = excluded_paths.iter().cloned().collect::<Vec<_>>();
    clear_sqlite_current_rows_for_paths(relational, &cfg.repo.repo_id, &excluded_paths_vec)
        .await
        .context("purging excluded sqlite-vec current rows")?;

    Ok(snapshot.fingerprint)
}

pub(crate) fn current_scope_exclusions_fingerprint(repo_root: &Path) -> Result<String> {
    load_scope_exclusion_snapshot(repo_root).map(|snapshot| snapshot.fingerprint)
}

fn load_scope_exclusion_snapshot(repo_root: &Path) -> Result<ScopeExclusionSnapshot> {
    let policy = discover_repo_policy_optional(repo_root)
        .with_context(|| format!("loading repo policy from {}", repo_root.display()))?;
    let policy_root = policy.root.unwrap_or_else(|| repo_root.to_path_buf());
    let policy_root = policy_root.canonicalize().unwrap_or(policy_root);
    let exclusions = resolve_repo_policy_scope_exclusions(&policy.scope, &policy_root)
        .context("resolving [scope] exclusions for exclusion reconciliation")?;
    let matcher = load_repo_exclusion_matcher(repo_root)?;
    Ok(ScopeExclusionSnapshot {
        fingerprint: scope_exclusions_fingerprint(&exclusions)?,
        matcher,
    })
}

fn scope_exclusions_fingerprint(exclusions: &RepoPolicyScopeExclusions) -> Result<String> {
    let mut exclude = exclusions.exclude.clone();
    exclude.sort();
    exclude.dedup();

    let mut exclude_from = exclusions.exclude_from.clone();
    exclude_from.sort();
    exclude_from.dedup();

    let mut referenced_files = exclusions.referenced_files.clone();
    referenced_files.sort_by(|left, right| {
        left.resolved_path
            .cmp(&right.resolved_path)
            .then_with(|| left.configured_path.cmp(&right.configured_path))
    });

    let value = Value::Object(Map::from_iter([
        (
            "exclude".into(),
            Value::Array(exclude.into_iter().map(Value::String).collect()),
        ),
        (
            "exclude_from".into(),
            Value::Array(exclude_from.into_iter().map(Value::String).collect()),
        ),
        (
            "referenced_files".into(),
            Value::Array(
                referenced_files
                    .into_iter()
                    .map(|reference| {
                        Value::Object(Map::from_iter([
                            (
                                "configured_path".into(),
                                Value::String(reference.configured_path),
                            ),
                            (
                                "resolved_path".into(),
                                Value::String(
                                    reference.resolved_path.to_string_lossy().to_string(),
                                ),
                            ),
                            ("content".into(), Value::String(reference.content)),
                        ]))
                    })
                    .collect(),
            ),
        ),
    ]));
    let canonical = serde_json::to_vec(&value)
        .context("serialising canonical scope exclusions snapshot for fingerprinting")?;
    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    Ok(hex::encode(hasher.finalize()))
}

async fn load_excluded_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    matcher: &RepoExclusionMatcher,
) -> Result<BTreeSet<String>> {
    let rows = query_rows_all(relational, &stored_repo_paths_sql(repo_id)).await?;
    let mut paths = BTreeSet::new();
    for row in rows {
        let Some(path) = row
            .as_object()
            .and_then(|row| row.get("path"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if matcher.excludes_repo_relative_path(path) {
            paths.insert(path.to_string());
        }
    }
    Ok(paths)
}

async fn load_distinct_string_set(
    relational: &RelationalStorage,
    sql: &str,
    column: &str,
) -> Result<BTreeSet<String>> {
    let mut values = BTreeSet::new();
    for row in query_rows_all(relational, sql).await? {
        if let Some(value) = row
            .as_object()
            .and_then(|row| row.get(column))
            .and_then(Value::as_str)
        {
            values.insert(value.to_string());
        }
    }
    Ok(values)
}

async fn query_rows_all(relational: &RelationalStorage, sql: &str) -> Result<Vec<Value>> {
    let mut rows = relational.query_rows(sql).await?;
    if relational.remote_client().is_some() {
        rows.extend(relational.query_rows_remote(sql).await?);
    }
    Ok(rows)
}

fn stored_repo_paths_sql(repo_id: &str) -> String {
    let repo_id = esc_pg(repo_id);
    format!(
        "SELECT DISTINCT path FROM current_file_state WHERE repo_id = '{repo_id}' \
UNION \
SELECT DISTINCT path FROM file_state WHERE repo_id = '{repo_id}' \
UNION \
SELECT DISTINCT path FROM artefacts_current WHERE repo_id = '{repo_id}' \
UNION \
SELECT DISTINCT path FROM artefact_edges_current WHERE repo_id = '{repo_id}' \
UNION \
SELECT DISTINCT path FROM artefact_snapshots WHERE repo_id = '{repo_id}' \
UNION \
SELECT DISTINCT path_before AS path FROM checkpoint_files WHERE repo_id = '{repo_id}' AND path_before IS NOT NULL \
UNION \
SELECT DISTINCT path_after AS path FROM checkpoint_files WHERE repo_id = '{repo_id}' AND path_after IS NOT NULL \
UNION \
SELECT DISTINCT copy_source_path AS path FROM checkpoint_files WHERE repo_id = '{repo_id}' AND copy_source_path IS NOT NULL"
    )
}

fn excluded_artefact_ids_sql(repo_id: &str, paths: &BTreeSet<String>) -> String {
    let repo_id = esc_pg(repo_id);
    let paths = quoted_sql_list(paths).unwrap_or_default();
    format!(
        "SELECT DISTINCT artefact_id FROM artefact_snapshots WHERE repo_id = '{repo_id}' AND path IN ({paths}) \
UNION \
SELECT DISTINCT artefact_id FROM artefacts_current WHERE repo_id = '{repo_id}' AND path IN ({paths})"
    )
}

fn excluded_blob_shas_sql(repo_id: &str, paths: &BTreeSet<String>) -> String {
    let repo_id = esc_pg(repo_id);
    let paths = quoted_sql_list(paths).unwrap_or_default();
    format!(
        "SELECT DISTINCT blob_sha FROM file_state WHERE repo_id = '{repo_id}' AND path IN ({paths}) \
UNION \
SELECT DISTINCT blob_sha FROM artefact_snapshots WHERE repo_id = '{repo_id}' AND path IN ({paths}) \
UNION \
SELECT DISTINCT blob_sha_before AS blob_sha FROM checkpoint_files WHERE repo_id = '{repo_id}' AND path_before IN ({paths}) AND blob_sha_before IS NOT NULL \
UNION \
SELECT DISTINCT blob_sha_after AS blob_sha FROM checkpoint_files WHERE repo_id = '{repo_id}' AND path_after IN ({paths}) AND blob_sha_after IS NOT NULL \
UNION \
SELECT DISTINCT copy_source_blob_sha AS blob_sha FROM checkpoint_files WHERE repo_id = '{repo_id}' AND copy_source_path IN ({paths}) AND copy_source_blob_sha IS NOT NULL"
    )
}

fn excluded_content_ids_sql(repo_id: &str, paths: &BTreeSet<String>) -> String {
    let repo_id = esc_pg(repo_id);
    let paths = quoted_sql_list(paths).unwrap_or_default();
    format!(
        "SELECT DISTINCT head_content_id AS content_id FROM current_file_state WHERE repo_id = '{repo_id}' AND path IN ({paths}) AND head_content_id IS NOT NULL \
UNION \
SELECT DISTINCT index_content_id AS content_id FROM current_file_state WHERE repo_id = '{repo_id}' AND path IN ({paths}) AND index_content_id IS NOT NULL \
UNION \
SELECT DISTINCT worktree_content_id AS content_id FROM current_file_state WHERE repo_id = '{repo_id}' AND path IN ({paths}) AND worktree_content_id IS NOT NULL \
UNION \
SELECT DISTINCT effective_content_id AS content_id FROM current_file_state WHERE repo_id = '{repo_id}' AND path IN ({paths}) AND effective_content_id IS NOT NULL"
    )
}

fn build_purge_statements(
    repo_id: &str,
    paths: &BTreeSet<String>,
    artefact_ids: &BTreeSet<String>,
    blob_shas: &BTreeSet<String>,
    content_ids: &BTreeSet<String>,
) -> Vec<String> {
    let Some(paths_sql) = quoted_sql_list(paths) else {
        return Vec::new();
    };
    let repo_id = esc_pg(repo_id);
    let mut statements = vec![
        format!(
            "DELETE FROM checkpoint_files WHERE repo_id = '{repo_id}' AND (path_before IN ({paths_sql}) OR path_after IN ({paths_sql}) OR copy_source_path IN ({paths_sql}))"
        ),
        format!(
            "DELETE FROM current_file_state WHERE repo_id = '{repo_id}' AND path IN ({paths_sql})"
        ),
        format!(
            "DELETE FROM artefacts_current WHERE repo_id = '{repo_id}' AND path IN ({paths_sql})"
        ),
        format!(
            "DELETE FROM artefact_edges_current WHERE repo_id = '{repo_id}' AND path IN ({paths_sql})"
        ),
        format!("DELETE FROM file_state WHERE repo_id = '{repo_id}' AND path IN ({paths_sql})"),
        format!(
            "DELETE FROM artefact_snapshots WHERE repo_id = '{repo_id}' AND path IN ({paths_sql})"
        ),
        format!(
            "DELETE FROM symbol_semantics_current WHERE repo_id = '{repo_id}' AND path IN ({paths_sql})"
        ),
        format!(
            "DELETE FROM symbol_features_current WHERE repo_id = '{repo_id}' AND path IN ({paths_sql})"
        ),
        format!(
            "DELETE FROM symbol_embeddings_current WHERE repo_id = '{repo_id}' AND path IN ({paths_sql})"
        ),
    ];

    let artefact_ids_sql = quoted_sql_list(artefact_ids);
    let blob_shas_sql = quoted_sql_list(blob_shas);

    let historical_edge_conditions = join_or_conditions([
        artefact_ids_sql
            .as_ref()
            .map(|ids| format!("from_artefact_id IN ({ids})")),
        artefact_ids_sql
            .as_ref()
            .map(|ids| format!("to_artefact_id IN ({ids})")),
        blob_shas_sql
            .as_ref()
            .map(|ids| format!("blob_sha IN ({ids})")),
    ]);
    if let Some(conditions) = historical_edge_conditions {
        statements.push(format!(
            "DELETE FROM artefact_edges WHERE repo_id = '{repo_id}' AND ({conditions})"
        ));
    }

    if let Some(ids) = artefact_ids_sql.as_ref() {
        statements.push(format!(
            "DELETE FROM checkpoint_artefacts WHERE repo_id = '{repo_id}' AND (before_artefact_id IN ({ids}) OR after_artefact_id IN ({ids}))"
        ));
        statements.push(format!(
            "DELETE FROM checkpoint_artefact_lineage WHERE repo_id = '{repo_id}' AND (source_artefact_id IN ({ids}) OR dest_artefact_id IN ({ids}))"
        ));
        statements.push(format!(
            "DELETE FROM artefacts WHERE repo_id = '{repo_id}' AND artefact_id IN ({ids})"
        ));
    }

    let historical_semantic_conditions = join_or_conditions([artefact_ids_sql
        .as_ref()
        .map(|ids| format!("artefact_id IN ({ids})"))]);
    if let Some(conditions) = historical_semantic_conditions {
        statements.push(format!(
            "DELETE FROM symbol_semantics WHERE repo_id = '{repo_id}' AND ({conditions})"
        ));
        statements.push(format!(
            "DELETE FROM symbol_features WHERE repo_id = '{repo_id}' AND ({conditions})"
        ));
        statements.push(format!(
            "DELETE FROM symbol_embeddings WHERE repo_id = '{repo_id}' AND ({conditions})"
        ));
    }

    if let Some(content_ids_sql) = quoted_sql_list(content_ids) {
        let content_unreferenced = format!(
            "NOT EXISTS (SELECT 1 FROM current_file_state c WHERE c.repo_id = '{repo_id}' AND (\
c.head_content_id = content_cache.content_id \
OR c.index_content_id = content_cache.content_id \
OR c.worktree_content_id = content_cache.content_id \
OR c.effective_content_id = content_cache.content_id))"
        );
        let artefacts_unreferenced = format!(
            "NOT EXISTS (SELECT 1 FROM current_file_state c WHERE c.repo_id = '{repo_id}' AND (\
c.head_content_id = content_cache_artefacts.content_id \
OR c.index_content_id = content_cache_artefacts.content_id \
OR c.worktree_content_id = content_cache_artefacts.content_id \
OR c.effective_content_id = content_cache_artefacts.content_id))"
        );
        let edges_unreferenced = format!(
            "NOT EXISTS (SELECT 1 FROM current_file_state c WHERE c.repo_id = '{repo_id}' AND (\
c.head_content_id = content_cache_edges.content_id \
OR c.index_content_id = content_cache_edges.content_id \
OR c.worktree_content_id = content_cache_edges.content_id \
OR c.effective_content_id = content_cache_edges.content_id))"
        );
        statements.push(format!(
            "DELETE FROM content_cache_artefacts WHERE content_id IN ({content_ids_sql}) AND {artefacts_unreferenced}"
        ));
        statements.push(format!(
            "DELETE FROM content_cache_edges WHERE content_id IN ({content_ids_sql}) AND {edges_unreferenced}"
        ));
        statements.push(format!(
            "DELETE FROM content_cache WHERE content_id IN ({content_ids_sql}) AND {content_unreferenced}"
        ));
    }

    statements
}

fn quoted_sql_list(values: &BTreeSet<String>) -> Option<String> {
    if values.is_empty() {
        None
    } else {
        Some(
            values
                .iter()
                .map(|value| format!("'{}'", esc_pg(value)))
                .collect::<Vec<_>>()
                .join(", "),
        )
    }
}

fn join_or_conditions<const N: usize>(conditions: [Option<String>; N]) -> Option<String> {
    let conditions = conditions.into_iter().flatten().collect::<Vec<_>>();
    if conditions.is_empty() {
        None
    } else {
        Some(conditions.join(" OR "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::REPO_POLICY_FILE_NAME;

    #[test]
    fn scope_exclusion_fingerprint_changes_when_exclude_file_content_changes() {
        let repo = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(repo.path().join(".git")).expect("create .git");
        std::fs::write(
            repo.path().join(REPO_POLICY_FILE_NAME),
            "[scope]\nexclude = [\"target/**\"]\nexclude_from = [\".bitloopsignore\"]\n",
        )
        .expect("write repo policy");
        std::fs::write(repo.path().join(".bitloopsignore"), "dist/**\n").expect("write ignore");

        let first = load_scope_exclusion_snapshot(repo.path())
            .expect("load first exclusions snapshot")
            .fingerprint;

        std::fs::write(
            repo.path().join(".bitloopsignore"),
            "dist/**\ncoverage/**\n",
        )
        .expect("rewrite ignore");

        let second = load_scope_exclusion_snapshot(repo.path())
            .expect("load second exclusions snapshot")
            .fingerprint;

        assert_ne!(
            first, second,
            "exclude file content should change fingerprint"
        );
    }
}
