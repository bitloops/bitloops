use std::collections::BTreeSet;

use anyhow::Result;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::host::relational_store::{DefaultRelationalStore, RelationalStore};

#[derive(Debug, Clone, Default)]
pub(crate) struct EmbeddingFreshnessState {
    pub(crate) eligible_artefact_ids: BTreeSet<String>,
    pub(crate) fresh_code_artefact_ids: BTreeSet<String>,
    pub(crate) fresh_identity_artefact_ids: BTreeSet<String>,
    pub(crate) fresh_summary_artefact_ids: BTreeSet<String>,
}

impl EmbeddingFreshnessState {
    pub(crate) fn code_lane_completed_count(&self) -> u64 {
        self.code_lane_ready_artefact_ids().len() as u64
    }

    pub(crate) fn summary_embeddings_completed_count(&self) -> u64 {
        self.fresh_summary_artefact_ids.len() as u64
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
