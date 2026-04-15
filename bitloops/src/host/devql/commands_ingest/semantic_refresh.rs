use super::*;

pub(super) async fn select_active_code_embedding_state_for_repo(
    relational: &RelationalStorage,
    repo_id: &str,
    setup: &embeddings::EmbeddingSetup,
) -> Result<
    Option<
        crate::capability_packs::semantic_clones::embeddings::ActiveEmbeddingRepresentationState,
    >,
> {
    let states = load_current_repo_embedding_states(
        relational,
        repo_id,
        Some(
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        ),
    )
    .await?;
    Ok(states.into_iter().find(|state| state.setup == *setup))
}

pub(super) async fn run_semantic_features_refresh(
    capability_host: &crate::host::capability_host::DevqlCapabilityHost,
    relational: &RelationalStorage,
    payload: SemanticFeaturesRefreshPayload,
) -> Result<(
    semantic::SemanticFeatureIngestionStats,
    std::collections::BTreeMap<String, String>,
    bool,
)> {
    let result = capability_host
        .invoke_ingester_with_relational(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
            serde_json::to_value(&payload)?,
            Some(relational),
        )
        .await?;
    Ok((
        semantic::SemanticFeatureIngestionStats {
            upserted: result.payload["semantic_feature_rows_upserted"]
                .as_u64()
                .unwrap_or_default() as usize,
            skipped: result.payload["semantic_feature_rows_skipped"]
                .as_u64()
                .unwrap_or_default() as usize,
        },
        parse_string_map(&result.payload["input_hashes"]),
        result.payload["produced_enriched_semantics"]
            .as_bool()
            .unwrap_or(false),
    ))
}

#[derive(Debug, Clone, Default)]
pub(super) struct SymbolEmbeddingsRefreshOutcome {
    semantic_feature_rows_upserted: usize,
    semantic_feature_rows_skipped: usize,
    symbol_embedding_rows_upserted: usize,
    symbol_embedding_rows_skipped: usize,
    pub(super) clone_rebuild_recommended: bool,
    symbol_clone_edges_upserted: usize,
    symbol_clone_sources_scored: usize,
}

pub(super) async fn run_symbol_embeddings_refresh(
    capability_host: &crate::host::capability_host::DevqlCapabilityHost,
    relational: &RelationalStorage,
    payload: SymbolEmbeddingsRefreshPayload,
) -> Result<SymbolEmbeddingsRefreshOutcome> {
    let result = capability_host
        .invoke_ingester_with_relational(
            SEMANTIC_CLONES_CAPABILITY_ID,
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
            serde_json::to_value(&payload)?,
            Some(relational),
        )
        .await?;
    Ok(SymbolEmbeddingsRefreshOutcome {
        semantic_feature_rows_upserted: result.payload["semantic_feature_rows_upserted"]
            .as_u64()
            .unwrap_or_default() as usize,
        semantic_feature_rows_skipped: result.payload["semantic_feature_rows_skipped"]
            .as_u64()
            .unwrap_or_default() as usize,
        symbol_embedding_rows_upserted: result.payload["symbol_embedding_rows_upserted"]
            .as_u64()
            .unwrap_or_default() as usize,
        symbol_embedding_rows_skipped: result.payload["symbol_embedding_rows_skipped"]
            .as_u64()
            .unwrap_or_default() as usize,
        clone_rebuild_recommended: result.payload["clone_rebuild_recommended"]
            .as_bool()
            .unwrap_or(false),
        symbol_clone_edges_upserted: result.payload["symbol_clone_edges_upserted"]
            .as_u64()
            .unwrap_or_default() as usize,
        symbol_clone_sources_scored: result.payload["symbol_clone_sources_scored"]
            .as_u64()
            .unwrap_or_default() as usize,
    })
}

pub(super) fn apply_symbol_embedding_refresh_counts(
    counters: &mut IngestionCounters,
    outcome: &SymbolEmbeddingsRefreshOutcome,
) {
    counters.semantic_feature_rows_upserted += outcome.semantic_feature_rows_upserted;
    counters.semantic_feature_rows_skipped += outcome.semantic_feature_rows_skipped;
    counters.symbol_embedding_rows_upserted += outcome.symbol_embedding_rows_upserted;
    counters.symbol_embedding_rows_skipped += outcome.symbol_embedding_rows_skipped;
    counters.symbol_clone_edges_upserted += outcome.symbol_clone_edges_upserted;
    counters.symbol_clone_sources_scored += outcome.symbol_clone_sources_scored;
}

fn parse_string_map(value: &serde_json::Value) -> std::collections::BTreeMap<String, String> {
    value
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) async fn rebuild_active_clone_edges(
    capability_host: &crate::host::capability_host::DevqlCapabilityHost,
    relational: &RelationalStorage,
) -> Result<(usize, usize)> {
    let clone_ingest = capability_host
        .invoke_ingester_with_relational(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
            json!({}),
            Some(relational),
        )
        .await
        .with_context(|| {
            format!(
                "running capability ingester `{SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID}` for `{SEMANTIC_CLONES_CAPABILITY_ID}`"
            )
        })?;

    Ok((
        clone_ingest.payload["symbol_clone_edges_upserted"]
            .as_u64()
            .unwrap_or_default() as usize,
        clone_ingest.payload["symbol_clone_sources_scored"]
            .as_u64()
            .unwrap_or_default() as usize,
    ))
}
