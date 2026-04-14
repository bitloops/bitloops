use anyhow::Result;
use serde_json::Value;

use super::storage::build_semantic_get_summary_sql;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::RelationalStorage;

pub(super) fn ensure_required_llm_summary_output(
    rows: &semantic::SemanticFeatureRows,
    summary_provider: &dyn semantic::SemanticSummaryProvider,
) -> Result<()> {
    if !summary_provider.requires_model_output() || rows.semantics.is_llm_enriched() {
        return Ok(());
    }

    anyhow::bail!(
        "configured semantic summary provider returned no model-backed summary for artefact `{}`",
        rows.semantics.artefact_id
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemanticSummarySnapshot {
    pub semantic_features_input_hash: String,
    pub summary: String,
    pub llm_summary: Option<String>,
    pub source_model: Option<String>,
}

impl SemanticSummarySnapshot {
    pub(crate) fn is_llm_enriched(&self) -> bool {
        self.llm_summary
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || self
                .source_model
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    }
}

pub(crate) async fn load_semantic_summary_snapshot(
    relational: &RelationalStorage,
    artefact_id: &str,
) -> Result<Option<SemanticSummarySnapshot>> {
    let rows = relational
        .query_rows(&build_semantic_get_summary_sql(artefact_id))
        .await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };

    let Some(input_hash) = row
        .get("semantic_features_input_hash")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let Some(summary) = row
        .get("summary")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let llm_summary = row
        .get("llm_summary")
        .and_then(Value::as_str)
        .map(str::to_string);
    let source_model = row
        .get("source_model")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(Some(SemanticSummarySnapshot {
        semantic_features_input_hash: input_hash,
        summary,
        llm_summary,
        source_model,
    }))
}
