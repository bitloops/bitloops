use std::collections::BTreeSet;

use crate::host::capability_host::CurrentStateConsumerRequest;
use crate::host::devql::RelationalStorage;

const CURRENT_PROJECTION_CLEAR_PATH_CHUNK_SIZE: usize = 500;

pub(super) fn collect_affected_paths(request: &CurrentStateConsumerRequest) -> BTreeSet<String> {
    request
        .affected_paths
        .iter()
        .cloned()
        .chain(request.file_upserts.iter().map(|file| file.path.clone()))
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
    let paths = affected_paths.iter().cloned().collect::<Vec<_>>();
    for chunk in paths.chunks(CURRENT_PROJECTION_CLEAR_PATH_CHUNK_SIZE) {
        let chunk_paths = chunk.to_vec();
        super::super::clear_current_semantic_feature_rows_for_paths(
            relational,
            repo_id,
            &chunk_paths,
        )
        .await?;
        super::super::clear_current_symbol_embedding_rows_for_paths(
            relational,
            repo_id,
            &chunk_paths,
        )
        .await?;
    }
    Ok(paths.len())
}

pub(super) async fn current_repo_backfill_artefact_ids(
    relational: &RelationalStorage,
    repo_id: &str,
) -> anyhow::Result<Vec<String>> {
    let repo_id_sql = crate::host::devql::esc_pg(repo_id);
    Ok(relational
        .query_rows(&format!(
            "SELECT DISTINCT a.artefact_id \
             FROM artefacts_current a \
             JOIN current_file_state cfs \
               ON cfs.repo_id = a.repo_id \
              AND cfs.path = a.path \
             WHERE a.repo_id = '{repo_id_sql}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
             ORDER BY a.path ASC, COALESCE(a.symbol_fqn, a.path) ASC, a.artefact_id ASC",
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
