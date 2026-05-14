use std::collections::BTreeSet;
use std::path::Path;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::runtime_config::embedding_slot_for_representation;
use crate::config::resolve_semantic_clones_config_for_repo;
use crate::daemon::types::InitSessionRecord;
use crate::host::relational_store::DefaultRelationalStore;

use super::embedding_freshness::{
    EmbeddingFreshnessCountSelection, load_embedding_freshness_counts, query_progress_count,
};
use super::stats::{RuntimeLaneProgressState, SessionWorkplaneStats, SummaryFreshnessState};
use super::types::InitRuntimeLaneProgressView;

const CURRENT_SUMMARY_SEMANTICS_TABLE: &str = "symbol_semantics_current";

#[cfg(test)]
static RUNTIME_LANE_PROGRESS_LOADS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn reset_runtime_lane_progress_load_count() {
    RUNTIME_LANE_PROGRESS_LOADS.store(0, Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) fn runtime_lane_progress_load_count() -> usize {
    RUNTIME_LANE_PROGRESS_LOADS.load(Ordering::SeqCst)
}

pub(crate) fn load_runtime_lane_progress(
    repo_root: &Path,
    repo_id: &str,
    session: &InitSessionRecord,
    _stats: &SessionWorkplaneStats,
    summary_in_memory_completed: u64,
) -> Result<RuntimeLaneProgressState> {
    #[cfg(test)]
    RUNTIME_LANE_PROGRESS_LOADS.fetch_add(1, Ordering::SeqCst);

    let semantic_clones = resolve_semantic_clones_config_for_repo(repo_root);
    let code_embeddings_enabled =
        embedding_slot_for_representation(&semantic_clones, EmbeddingRepresentationKind::Code)
            .is_some();
    let summary_embeddings_enabled =
        embedding_slot_for_representation(&semantic_clones, EmbeddingRepresentationKind::Summary)
            .is_some();
    let needs_code_embeddings = session.selections.run_code_embeddings && code_embeddings_enabled;
    let needs_summaries = session.selections.run_summaries;
    let needs_summary_embeddings =
        session.selections.run_summary_embeddings && summary_embeddings_enabled;

    if !needs_code_embeddings && !needs_summaries && !needs_summary_embeddings {
        return Ok(RuntimeLaneProgressState::default());
    }

    let relational =
        DefaultRelationalStore::open_primary_for_repo_root_preferring_bound_config(repo_root)?;
    let embedding_freshness = load_embedding_freshness_counts(
        &relational,
        repo_id,
        EmbeddingFreshnessCountSelection {
            code_lane: needs_code_embeddings,
            summary_embeddings: needs_summary_embeddings,
        },
    )?;
    let total_eligible = embedding_freshness.eligible;
    let summaries_completed = if needs_summaries {
        count_current_model_backed_summary_artefacts(&relational, repo_id)?
    } else {
        0
    };
    let code_embeddings_total = u64::from(needs_code_embeddings) * total_eligible;
    let summary_embeddings_total = u64::from(needs_summary_embeddings) * total_eligible;
    let code_embeddings_completed = embedding_freshness
        .code_lane_completed
        .min(code_embeddings_total);
    let summaries_total = if session.selections.run_summaries {
        total_eligible
    } else {
        0
    };
    let summaries_completed = summaries_completed.min(summaries_total);
    let summary_embeddings_completed = embedding_freshness
        .fresh_summary
        .min(summary_embeddings_total);
    let summary_in_memory_completed =
        summary_in_memory_completed.min(summaries_total.saturating_sub(summaries_completed));

    Ok(RuntimeLaneProgressState {
        code_embeddings: (needs_code_embeddings && code_embeddings_total > 0).then(|| {
            InitRuntimeLaneProgressView {
                completed: code_embeddings_completed,
                in_memory_completed: 0,
                total: code_embeddings_total,
                remaining: code_embeddings_total.saturating_sub(code_embeddings_completed),
            }
        }),
        summaries: (needs_summaries && summaries_total > 0).then(|| InitRuntimeLaneProgressView {
            completed: summaries_completed,
            in_memory_completed: summary_in_memory_completed,
            total: summaries_total,
            remaining: summaries_total.saturating_sub(summaries_completed),
        }),
        summary_embeddings: (needs_summary_embeddings && summary_embeddings_total > 0).then(|| {
            InitRuntimeLaneProgressView {
                completed: summary_embeddings_completed,
                in_memory_completed: 0,
                total: summary_embeddings_total,
                remaining: summary_embeddings_total.saturating_sub(summary_embeddings_completed),
            }
        }),
    })
}

fn count_current_model_backed_summary_artefacts(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<u64> {
    query_progress_count(
        relational,
        &format!(
            "SELECT COUNT(*) AS total FROM ({}) fresh",
            fresh_model_backed_summary_artefacts_sql(repo_id),
        ),
    )
}

pub(crate) fn load_summary_freshness_state(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<SummaryFreshnessState> {
    let eligible_artefact_ids =
        query_progress_ids(relational, &eligible_current_summary_artefacts_sql(repo_id))?;
    let fresh_model_backed_artefact_ids = query_progress_ids(
        relational,
        &fresh_model_backed_summary_artefacts_sql(repo_id),
    )?;

    Ok(SummaryFreshnessState {
        eligible_artefact_ids,
        fresh_model_backed_artefact_ids,
    })
}

fn eligible_current_summary_artefacts_sql(repo_id: &str) -> String {
    format!(
        "SELECT DISTINCT a.artefact_id \
         FROM artefacts_current a \
         JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
         WHERE a.repo_id = '{}' \
           AND cfs.analysis_mode = 'code' \
           AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import'",
        escape_sql_string(repo_id),
    )
}

fn fresh_model_backed_summary_artefacts_sql(repo_id: &str) -> String {
    let repo_id = escape_sql_string(repo_id);
    format!(
        "SELECT DISTINCT a.artefact_id \
         FROM artefacts_current a \
         JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
         WHERE a.repo_id = '{repo_id}' \
           AND cfs.analysis_mode = 'code' \
           AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
           AND ( \
                EXISTS ( \
                    SELECT 1 \
                    FROM symbol_features_current f \
                    JOIN {CURRENT_SUMMARY_SEMANTICS_TABLE} s \
                      ON s.repo_id = f.repo_id \
                     AND s.artefact_id = f.artefact_id \
                     AND s.content_id = f.content_id \
                    WHERE f.repo_id = a.repo_id \
                      AND f.artefact_id = a.artefact_id \
                      AND f.content_id = a.content_id \
                      AND s.semantic_features_input_hash = f.semantic_features_input_hash \
                      AND ( \
                           (s.llm_summary IS NOT NULL AND TRIM(s.llm_summary) <> '') \
                           OR (s.source_model IS NOT NULL AND TRIM(s.source_model) <> '') \
                      ) \
                ) \
                OR EXISTS ( \
                    SELECT 1 \
                    FROM symbol_features f \
                    JOIN symbol_semantics s \
                      ON s.repo_id = f.repo_id \
                     AND s.artefact_id = f.artefact_id \
                     AND s.blob_sha = f.blob_sha \
                    WHERE f.repo_id = a.repo_id \
                      AND f.artefact_id = a.artefact_id \
                      AND f.blob_sha = a.content_id \
                      AND s.semantic_features_input_hash = f.semantic_features_input_hash \
                      AND ( \
                           (s.llm_summary IS NOT NULL AND TRIM(s.llm_summary) <> '') \
                           OR (s.source_model IS NOT NULL AND TRIM(s.source_model) <> '') \
                      ) \
                ) \
           )",
    )
}

fn query_progress_ids(relational: &DefaultRelationalStore, sql: &str) -> Result<BTreeSet<String>> {
    match relational.query_rows_primary_blocking(sql) {
        Ok(rows) => Ok(rows
            .into_iter()
            .filter_map(|row| {
                row.as_object()
                    .and_then(|object| object.values().next())
                    .cloned()
            })
            .filter_map(|value| value.as_str().map(ToOwned::to_owned))
            .collect()),
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::*;
    use crate::config::resolve_store_backend_config_for_repo;
    use crate::daemon::{InitSessionRecord, StartInitSessionSelections};
    use crate::host::devql::{RelationalPrimaryBackend, RelationalStorage};

    #[test]
    fn code_embeddings_only_progress_does_not_query_summary_counts() {
        let repo = tempdir().expect("temp repo");
        let config_path = crate::test_support::git_fixtures::write_test_daemon_config(repo.path());
        let mut config = fs::OpenOptions::new()
            .append(true)
            .open(&config_path)
            .expect("open daemon config");
        writeln!(
            config,
            r#"
[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
code_embeddings = "local_code"
"#
        )
        .expect("append semantic clones config");

        initialise_progress_tables(repo.path());

        let session = InitSessionRecord {
            init_session_id: "init-session-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: repo.path().to_path_buf(),
            daemon_config_root: repo.path().to_path_buf(),
            selections: StartInitSessionSelections {
                run_sync: true,
                run_ingest: true,
                run_code_embeddings: true,
                run_summaries: false,
                run_summary_embeddings: false,
                ingest_backfill: None,
                embeddings_bootstrap: None,
                summaries_bootstrap: None,
            },
            initial_sync_task_id: None,
            initial_sync_terminal: None,
            ingest_task_id: None,
            ingest_terminal: None,
            embeddings_bootstrap_task_id: None,
            embeddings_bootstrap_terminal: None,
            summary_bootstrap_task_id: None,
            summary_bootstrap_terminal: None,
            follow_up_sync_required: false,
            follow_up_sync_task_id: None,
            follow_up_sync_terminal: None,
            next_completion_seq: 0,
            initial_sync_completion_seq: None,
            embeddings_bootstrap_completion_seq: None,
            summary_bootstrap_completion_seq: None,
            follow_up_sync_completion_seq: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            terminal_status: None,
            terminal_error: None,
        };

        let progress = load_runtime_lane_progress(
            repo.path(),
            "repo-1",
            &session,
            &SessionWorkplaneStats::default(),
            0,
        )
        .expect("load code embeddings progress without touching summary count tables");

        assert!(progress.summaries.is_none());
    }

    fn initialise_progress_tables(repo_root: &Path) {
        let backend =
            resolve_store_backend_config_for_repo(repo_root).expect("resolve store backend");
        let sqlite_path = backend
            .relational
            .resolve_sqlite_db_path_for_repo(repo_root)
            .expect("resolve sqlite path");
        fs::create_dir_all(sqlite_path.parent().expect("sqlite parent"))
            .expect("create sqlite dir");
        let conn = Connection::open(sqlite_path).expect("open sqlite");
        conn.execute_batch(
            "
            CREATE TABLE artefacts_current (
                repo_id TEXT,
                path TEXT,
                artefact_id TEXT,
                content_id TEXT,
                canonical_kind TEXT,
                language_kind TEXT
            );
            CREATE TABLE current_file_state (
                repo_id TEXT,
                path TEXT,
                analysis_mode TEXT
            );
            CREATE TABLE semantic_clone_embedding_setup_state (
                repo_id TEXT,
                representation_kind TEXT,
                setup_fingerprint TEXT,
                provider TEXT,
                model TEXT,
                dimension INTEGER
            );
            CREATE TABLE symbol_embeddings_current (
                repo_id TEXT,
                artefact_id TEXT,
                content_id TEXT,
                setup_fingerprint TEXT,
                provider TEXT,
                model TEXT,
                dimension INTEGER,
                representation_kind TEXT
            );
            CREATE TABLE symbol_features_current (
                repo_id TEXT,
                artefact_id TEXT,
                content_id TEXT,
                semantic_features_input_hash TEXT
            );
            CREATE TABLE symbol_semantics_current (
                repo_id TEXT,
                artefact_id TEXT,
                content_id TEXT
            );
            CREATE TABLE symbol_features (
                repo_id TEXT,
                artefact_id TEXT,
                blob_sha TEXT,
                semantic_features_input_hash TEXT
            );
            CREATE TABLE symbol_semantics (
                repo_id TEXT,
                artefact_id TEXT,
                blob_sha TEXT,
                semantic_features_input_hash TEXT,
                llm_summary TEXT,
                source_model TEXT
            );
            ",
        )
        .expect("initialise progress tables");
    }

    #[test]
    fn query_progress_ids_uses_primary_backend_when_postgres_is_configured() {
        let temp = tempdir().expect("temp dir");
        let db_path = temp.path().join("relational.sqlite");
        let conn = Connection::open(&db_path).expect("create sqlite file");
        conn.execute_batch(
            "CREATE TABLE progress_rows (artefact_id TEXT);
             INSERT INTO progress_rows (artefact_id) VALUES ('artefact-1');",
        )
        .expect("seed sqlite progress rows");
        let relational = DefaultRelationalStore::from_inner(
            RelationalStorage::primary_backend_with_dsn_for_tests(
                db_path,
                RelationalPrimaryBackend::Postgres,
                Some("postgres://not a valid dsn".to_string()),
            ),
        );

        let err = query_progress_ids(&relational, "SELECT artefact_id FROM progress_rows")
            .expect_err("configured Postgres primary backend should be queried");

        assert!(
            err.to_string()
                .contains("querying primary relational Postgres rows")
        );
    }
}
