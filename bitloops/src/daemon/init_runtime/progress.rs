use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::runtime_config::embedding_slot_for_representation;
use crate::config::resolve_semantic_clones_config_for_repo;
use crate::daemon::types::InitSessionRecord;
use crate::host::relational_store::{DefaultRelationalStore, RelationalStore};

use super::lanes::derive_embeddings_completed_count;
use super::stats::{RuntimeLaneProgressState, SessionWorkplaneStats, SummaryFreshnessState};
use super::types::InitRuntimeLaneProgressView;

const CURRENT_CODE_EMBEDDINGS_TABLE: &str = "symbol_embeddings_current";
const CURRENT_SUMMARY_SEMANTICS_TABLE: &str = "symbol_semantics_current";

pub(crate) fn load_runtime_lane_progress(
    repo_root: &Path,
    repo_id: &str,
    session: &InitSessionRecord,
    stats: &SessionWorkplaneStats,
    summary_in_memory_completed: u64,
) -> Result<RuntimeLaneProgressState> {
    let relational =
        DefaultRelationalStore::open_local_for_repo_root_preferring_bound_config(repo_root)?;
    let total_eligible = count_eligible_current_artefacts(&relational, repo_id)?;
    let summaries_completed = count_current_model_backed_summary_artefacts(&relational, repo_id)?;
    let code_embeddings_completed =
        count_current_embedding_artefacts(&relational, repo_id, "code")?;
    let summary_embeddings_completed =
        count_current_embedding_artefacts(&relational, repo_id, "summary")?;
    let semantic_clones = resolve_semantic_clones_config_for_repo(repo_root);

    let code_embeddings_enabled =
        embedding_slot_for_representation(&semantic_clones, EmbeddingRepresentationKind::Code)
            .is_some();
    let summary_embeddings_enabled =
        embedding_slot_for_representation(&semantic_clones, EmbeddingRepresentationKind::Summary)
            .is_some();
    let code_embeddings_total = u64::from(code_embeddings_enabled) * total_eligible;
    let summary_embeddings_total = u64::from(summary_embeddings_enabled) * total_eligible;
    let embeddings_total = code_embeddings_total + summary_embeddings_total;
    let embeddings_completed = derive_embeddings_completed_count(
        code_embeddings_total,
        code_embeddings_completed,
        stats.code_embedding_jobs.counts,
        summary_embeddings_total,
        summary_embeddings_completed,
        summaries_completed,
        stats.summary_embedding_jobs.counts,
    )
    .min(embeddings_total);
    let summaries_total = if session.selections.summaries_bootstrap.is_some() {
        total_eligible
    } else {
        0
    };
    let summaries_completed = summaries_completed.min(summaries_total);
    let summary_in_memory_completed =
        summary_in_memory_completed.min(summaries_total.saturating_sub(summaries_completed));

    Ok(RuntimeLaneProgressState {
        embeddings: (session.selections.embeddings_bootstrap.is_some() && embeddings_total > 0)
            .then(|| InitRuntimeLaneProgressView {
                completed: embeddings_completed,
                in_memory_completed: 0,
                total: embeddings_total,
                remaining: embeddings_total.saturating_sub(embeddings_completed),
            }),
        summaries: (session.selections.summaries_bootstrap.is_some() && summaries_total > 0).then(
            || InitRuntimeLaneProgressView {
                completed: summaries_completed,
                in_memory_completed: summary_in_memory_completed,
                total: summaries_total,
                remaining: summaries_total.saturating_sub(summaries_completed),
            },
        ),
    })
}

fn count_eligible_current_artefacts(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<u64> {
    query_progress_count(
        relational,
        &format!(
            "SELECT COUNT(DISTINCT a.artefact_id) AS total \
             FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             WHERE a.repo_id = '{}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import'",
            escape_sql_string(repo_id),
        ),
    )
}

fn count_current_embedding_artefacts(
    relational: &DefaultRelationalStore,
    repo_id: &str,
    representation_kind: &str,
) -> Result<u64> {
    query_progress_count(
        relational,
        &format!(
            "SELECT COUNT(DISTINCT a.artefact_id) AS total \
             FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             JOIN {CURRENT_CODE_EMBEDDINGS_TABLE} e ON e.repo_id = a.repo_id AND e.artefact_id = a.artefact_id \
             WHERE a.repo_id = '{}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
               AND LOWER(COALESCE(e.representation_kind, 'code')) = '{}'",
            escape_sql_string(repo_id),
            escape_sql_string(representation_kind),
        ),
    )
}

fn count_current_model_backed_summary_artefacts(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<u64> {
    query_progress_count(
        relational,
        &format!(
            "SELECT COUNT(DISTINCT a.artefact_id) AS total \
             FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             JOIN symbol_features_current f ON f.repo_id = a.repo_id AND f.artefact_id = a.artefact_id AND f.content_id = a.content_id \
             JOIN {CURRENT_SUMMARY_SEMANTICS_TABLE} s ON s.repo_id = a.repo_id AND s.artefact_id = a.artefact_id AND s.content_id = a.content_id \
             WHERE a.repo_id = '{}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
               AND s.semantic_features_input_hash = f.semantic_features_input_hash \
               AND ( \
                    (s.llm_summary IS NOT NULL AND TRIM(s.llm_summary) <> '') \
                    OR (s.source_model IS NOT NULL AND TRIM(s.source_model) <> '') \
               )",
            escape_sql_string(repo_id),
        ),
    )
}

pub(crate) fn load_summary_freshness_state(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<SummaryFreshnessState> {
    let eligible_artefact_ids = query_progress_ids(
        relational,
        &format!(
            "SELECT DISTINCT a.artefact_id \
             FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             WHERE a.repo_id = '{}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import'",
            escape_sql_string(repo_id),
        ),
    )?;
    let fresh_model_backed_artefact_ids = query_progress_ids(
        relational,
        &format!(
            "SELECT DISTINCT a.artefact_id \
             FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             JOIN symbol_features_current f ON f.repo_id = a.repo_id AND f.artefact_id = a.artefact_id AND f.content_id = a.content_id \
             JOIN {CURRENT_SUMMARY_SEMANTICS_TABLE} s ON s.repo_id = a.repo_id AND s.artefact_id = a.artefact_id AND s.content_id = a.content_id \
             WHERE a.repo_id = '{}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
               AND s.semantic_features_input_hash = f.semantic_features_input_hash \
               AND ( \
                    (s.llm_summary IS NOT NULL AND TRIM(s.llm_summary) <> '') \
                    OR (s.source_model IS NOT NULL AND TRIM(s.source_model) <> '') \
               )",
            escape_sql_string(repo_id),
        ),
    )?;

    Ok(SummaryFreshnessState {
        eligible_artefact_ids,
        fresh_model_backed_artefact_ids,
    })
}

fn query_progress_count(relational: &DefaultRelationalStore, sql: &str) -> Result<u64> {
    let sqlite = relational.local_sqlite_pool()?;
    let count =
        sqlite.with_connection(|conn| Ok(conn.query_row(sql, [], |row| row.get::<_, i64>(0))?));
    match count {
        Ok(value) => Ok(u64::try_from(value).unwrap_or_default()),
        Err(err) if missing_progress_table(&err) => Ok(0),
        Err(err) => Err(err),
    }
}

fn query_progress_ids(relational: &DefaultRelationalStore, sql: &str) -> Result<BTreeSet<String>> {
    let sqlite = relational.local_sqlite_pool()?;
    let values = sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut ids = BTreeSet::new();
        for row in rows {
            ids.insert(row?);
        }
        Ok(ids)
    });
    match values {
        Ok(ids) => Ok(ids),
        Err(err) if missing_progress_table(&err) => Ok(BTreeSet::new()),
        Err(err) => Err(err),
    }
}

fn missing_progress_table(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("no such table:") || message.contains("does not exist")
}

fn escape_sql_string(value: &str) -> String {
    value.replace('\'', "''")
}
