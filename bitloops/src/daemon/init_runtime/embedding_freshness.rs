use std::collections::BTreeSet;

use anyhow::Result;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::host::devql::RelationalStorageRole;
use crate::host::relational_store::DefaultRelationalStore;

#[derive(Debug, Clone, Default)]
pub(crate) struct EmbeddingFreshnessState {
    pub(crate) eligible_artefact_ids: BTreeSet<String>,
    pub(crate) fresh_code_artefact_ids: BTreeSet<String>,
    pub(crate) fresh_identity_artefact_ids: BTreeSet<String>,
    pub(crate) fresh_summary_artefact_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EmbeddingFreshnessCountSelection {
    pub(crate) code_lane: bool,
    pub(crate) summary_embeddings: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EmbeddingFreshnessCounts {
    pub(crate) eligible: u64,
    pub(crate) code_lane_completed: u64,
    pub(crate) fresh_summary: u64,
}

impl EmbeddingFreshnessState {
    #[cfg(test)]
    pub(crate) fn code_lane_completed_count(&self) -> u64 {
        self.code_lane_ready_artefact_ids().len() as u64
    }

    pub(crate) fn outstanding_work_item_count(
        &self,
        representation_kind: EmbeddingRepresentationKind,
    ) -> u64 {
        self.eligible_artefact_ids
            .iter()
            .filter(|artefact_id| {
                self.artefact_needs_representation_refresh(representation_kind, artefact_id)
            })
            .count() as u64
    }

    pub(crate) fn outstanding_work_item_count_for_artefacts(
        &self,
        representation_kind: EmbeddingRepresentationKind,
        artefact_ids: &[String],
    ) -> u64 {
        artefact_ids
            .iter()
            .filter(|artefact_id| {
                self.artefact_needs_representation_refresh(representation_kind, artefact_id)
            })
            .count() as u64
    }

    pub(crate) fn artefact_needs_representation_refresh(
        &self,
        representation_kind: EmbeddingRepresentationKind,
        artefact_id: &str,
    ) -> bool {
        self.eligible_artefact_ids.contains(artefact_id)
            && !self
                .fresh_artefact_ids(representation_kind)
                .contains(artefact_id)
    }

    #[cfg(test)]
    fn code_lane_ready_artefact_ids(&self) -> BTreeSet<String> {
        // The init code lane is only complete once both code and identity views are current.
        self.eligible_artefact_ids
            .intersection(&self.fresh_code_artefact_ids)
            .filter(|artefact_id| self.fresh_identity_artefact_ids.contains(*artefact_id))
            .cloned()
            .collect()
    }

    fn fresh_artefact_ids(
        &self,
        representation_kind: EmbeddingRepresentationKind,
    ) -> &BTreeSet<String> {
        match representation_kind {
            EmbeddingRepresentationKind::Code => &self.fresh_code_artefact_ids,
            EmbeddingRepresentationKind::Summary => &self.fresh_summary_artefact_ids,
            EmbeddingRepresentationKind::Identity => &self.fresh_identity_artefact_ids,
        }
    }
}

pub(crate) fn load_embedding_freshness_state(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<EmbeddingFreshnessState> {
    Ok(EmbeddingFreshnessState {
        eligible_artefact_ids: query_progress_ids(
            relational,
            &eligible_current_artefacts_sql(repo_id),
        )?,
        fresh_code_artefact_ids: query_progress_ids(
            relational,
            &fresh_embedding_artefacts_sql(repo_id, EmbeddingRepresentationKind::Code),
        )?,
        fresh_identity_artefact_ids: query_progress_ids(
            relational,
            &fresh_embedding_artefacts_sql(repo_id, EmbeddingRepresentationKind::Identity),
        )?,
        fresh_summary_artefact_ids: query_progress_ids(
            relational,
            &fresh_embedding_artefacts_sql(repo_id, EmbeddingRepresentationKind::Summary),
        )?,
    })
}

pub(crate) fn load_embedding_freshness_counts(
    relational: &DefaultRelationalStore,
    repo_id: &str,
    selection: EmbeddingFreshnessCountSelection,
) -> Result<EmbeddingFreshnessCounts> {
    let eligible_sql = eligible_current_artefacts_sql(repo_id);
    let fresh_code_sql = selection
        .code_lane
        .then(|| fresh_embedding_artefacts_sql(repo_id, EmbeddingRepresentationKind::Code));
    let fresh_identity_sql = selection
        .code_lane
        .then(|| fresh_embedding_artefacts_sql(repo_id, EmbeddingRepresentationKind::Identity));
    let fresh_summary_sql = selection
        .summary_embeddings
        .then(|| fresh_embedding_artefacts_sql(repo_id, EmbeddingRepresentationKind::Summary));

    let code_lane_completed = match (fresh_code_sql.as_deref(), fresh_identity_sql.as_deref()) {
        (Some(code_sql), Some(identity_sql)) => query_progress_count(
            relational,
            &fresh_code_and_identity_count_sql(code_sql, identity_sql),
        )?,
        _ => 0,
    };
    let fresh_summary = match fresh_summary_sql.as_deref() {
        Some(sql) => query_progress_count(relational, &count_sql(sql))?,
        None => 0,
    };

    Ok(EmbeddingFreshnessCounts {
        eligible: query_progress_count(relational, &count_sql(&eligible_sql))?,
        code_lane_completed,
        fresh_summary,
    })
}

pub(crate) fn parse_embedding_representation_kind(
    raw: &str,
) -> Option<EmbeddingRepresentationKind> {
    let raw = raw.trim();
    if EmbeddingRepresentationKind::Code
        .storage_values()
        .iter()
        .any(|value| raw.eq_ignore_ascii_case(value))
    {
        return Some(EmbeddingRepresentationKind::Code);
    }
    if EmbeddingRepresentationKind::Summary
        .storage_values()
        .iter()
        .any(|value| raw.eq_ignore_ascii_case(value))
    {
        return Some(EmbeddingRepresentationKind::Summary);
    }
    if EmbeddingRepresentationKind::Identity
        .storage_values()
        .iter()
        .any(|value| raw.eq_ignore_ascii_case(value))
    {
        return Some(EmbeddingRepresentationKind::Identity);
    }
    None
}

fn eligible_current_artefacts_sql(repo_id: &str) -> String {
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

fn fresh_embedding_artefacts_sql(
    repo_id: &str,
    representation_kind: EmbeddingRepresentationKind,
) -> String {
    format!(
        "SELECT DISTINCT a.artefact_id \
         FROM artefacts_current a \
         JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
         JOIN semantic_clone_embedding_setup_state st \
           ON st.repo_id = a.repo_id \
          AND LOWER(st.representation_kind) = '{}' \
         JOIN symbol_embeddings_current e \
           ON e.repo_id = a.repo_id \
          AND e.artefact_id = a.artefact_id \
          AND e.content_id = a.content_id \
         WHERE a.repo_id = '{}' \
           AND cfs.analysis_mode = 'code' \
           AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
           AND ({representation_predicate}) \
           AND e.setup_fingerprint = st.setup_fingerprint \
           AND e.provider = st.provider \
           AND e.model = st.model \
           AND e.dimension = st.dimension",
        escape_sql_string(&representation_kind.to_string()),
        escape_sql_string(repo_id),
        representation_predicate = representation_kind_sql_predicate(
            "LOWER(COALESCE(e.representation_kind, 'code'))",
            representation_kind
        ),
    )
}

fn representation_kind_sql_predicate(
    column: &str,
    representation_kind: EmbeddingRepresentationKind,
) -> String {
    representation_kind
        .storage_values()
        .iter()
        .map(|value| format!("{column} = '{}'", escape_sql_string(value)))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn query_progress_ids(relational: &DefaultRelationalStore, sql: &str) -> Result<BTreeSet<String>> {
    match relational.query_rows_for_role_blocking(RelationalStorageRole::CurrentProjection, sql) {
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

pub(crate) fn query_progress_count(relational: &DefaultRelationalStore, sql: &str) -> Result<u64> {
    match relational.query_rows_for_role_blocking(RelationalStorageRole::CurrentProjection, sql) {
        Ok(rows) => Ok(rows
            .first()
            .and_then(|row| row.as_object())
            .and_then(|object| object.values().next())
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
                    .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
            })
            .unwrap_or_default()),
        Err(err) if missing_progress_table(&err) => Ok(0),
        Err(err) => Err(err),
    }
}

fn count_sql(inner: &str) -> String {
    format!("SELECT COUNT(*) AS total FROM ({inner}) progress_rows")
}

fn fresh_code_and_identity_count_sql(fresh_code_sql: &str, fresh_identity_sql: &str) -> String {
    format!(
        "SELECT COUNT(*) AS total \
         FROM ({fresh_code_sql}) code_rows \
         JOIN ({fresh_identity_sql}) identity_rows \
           ON identity_rows.artefact_id = code_rows.artefact_id"
    )
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
    use tempfile::tempdir;

    use crate::host::devql::{RelationalPrimaryBackend, RelationalStorage};
    use crate::host::relational_store::DefaultRelationalStore;

    use super::*;

    #[test]
    fn count_sql_wraps_distinct_progress_query() {
        assert_eq!(
            count_sql("SELECT DISTINCT artefact_id FROM artefacts_current"),
            "SELECT COUNT(*) AS total FROM (SELECT DISTINCT artefact_id FROM artefacts_current) progress_rows"
        );
    }

    #[test]
    fn fresh_code_and_identity_count_sql_intersects_by_artefact_id() {
        let sql = fresh_code_and_identity_count_sql(
            "SELECT DISTINCT artefact_id FROM code_rows",
            "SELECT DISTINCT artefact_id FROM identity_rows",
        );

        assert!(sql.contains("identity_rows.artefact_id = code_rows.artefact_id"));
        assert!(sql.contains("SELECT COUNT(*) AS total"));
    }

    #[test]
    fn query_progress_count_reads_current_projection_from_local_sqlite_when_postgres_is_configured()
    {
        let temp = tempdir().expect("temp dir");
        let db_path = temp.path().join("relational.sqlite");
        let conn = rusqlite::Connection::open(&db_path).expect("create sqlite file");
        conn.execute("CREATE TABLE counts (total INTEGER)", [])
            .expect("create counts table");
        conn.execute("INSERT INTO counts (total) VALUES (7)", [])
            .expect("insert count");
        let relational = DefaultRelationalStore::from_inner(
            RelationalStorage::primary_backend_with_dsn_for_tests(
                db_path,
                RelationalPrimaryBackend::Postgres,
                Some("postgres://not a valid dsn".to_string()),
            ),
        );

        assert_eq!(
            query_progress_count(&relational, "SELECT total FROM counts").expect("query count"),
            7
        );
    }
}
