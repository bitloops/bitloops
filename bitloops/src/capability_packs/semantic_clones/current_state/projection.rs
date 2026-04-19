use std::collections::BTreeSet;

use crate::host::capability_host::CurrentStateConsumerRequest;
use crate::host::devql::RelationalStorage;

pub(super) fn collect_affected_paths(request: &CurrentStateConsumerRequest) -> BTreeSet<String> {
    request
        .file_upserts
        .iter()
        .map(|file| file.path.clone())
        .chain(request.file_removals.iter().map(|file| file.path.clone()))
        .chain(
            request
                .artefact_upserts
                .iter()
                .map(|artefact| artefact.path.clone()),
        )
        .chain(
            request
                .artefact_removals
                .iter()
                .map(|artefact| artefact.path.clone()),
        )
        .collect()
}

pub(super) async fn clear_current_projection_rows(
    relational: &RelationalStorage,
    repo_id: &str,
    affected_paths: &BTreeSet<String>,
) -> anyhow::Result<usize> {
    let mut cleared = 0usize;
    for path in affected_paths {
        super::super::clear_current_semantic_feature_rows_for_path(relational, repo_id, path)
            .await?;
        super::super::clear_current_symbol_embedding_rows_for_path(relational, repo_id, path)
            .await?;
        cleared += 1;
    }
    Ok(cleared)
}

pub(super) async fn current_repo_backfill_artefact_ids(
    relational: &RelationalStorage,
    repo_id: &str,
) -> anyhow::Result<Vec<String>> {
    let repo_id_sql = crate::host::devql::esc_pg(repo_id);
    Ok(relational
        .query_rows(&format!(
            "SELECT DISTINCT artefact_id \
             FROM artefacts_current \
             WHERE repo_id = '{repo_id_sql}' \
               AND LOWER(COALESCE(canonical_kind, COALESCE(language_kind, 'symbol'))) <> 'import' \
             ORDER BY path ASC, COALESCE(symbol_fqn, path) ASC, artefact_id ASC",
        ))
        .await?
        .into_iter()
        .filter_map(|row| {
            row.get("artefact_id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|artefact_id| !artefact_id.is_empty())
                .map(str::to_string)
        })
        .collect())
}
