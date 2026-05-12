use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context as _, Result, anyhow};
use async_graphql::Result as GraphqlResult;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use serde_json::Value;

use crate::artefact_query_planner::plan_graphql_artefact_query;
use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::embeddings::{
    EmbeddingRepresentationKind as SemanticEmbeddingRepresentationKind, EmbeddingSetup,
    resolve_embedding_setup,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    EmbeddingProviderMode, embeddings_enabled, resolve_embedding_provider,
    resolve_semantic_clones_config,
};
use crate::capability_packs::semantic_clones::vector_backend::{
    SemanticVectorBackend, SemanticVectorQuery,
};
use crate::graphql::types::{
    Artefact, CanonicalKind, DateTimeScalar,
    EmbeddingRepresentationKind as GraphqlEmbeddingRepresentationKind,
};
use crate::graphql::{DevqlGraphqlContext, ResolverScope, backend_error, bad_user_input_error};
use crate::host::devql::artefact_sql::build_filtered_artefacts_cte_sql;
use crate::host::devql::{RelationalStorage, esc_pg, sql_string_list_pg};
use crate::host::inference::EmbeddingInputType;
use crate::vector_search::{VectorSearchMode, normalized_cosine_similarity};

const DEFAULT_MIN_SEMANTIC_SCORE: f32 = 0.72;
const DEFAULT_RESULT_LIMIT: usize = 5;
const ANN_PREFILTER_MULTIPLIER: usize = 4;
const ANN_PREFILTER_MIN_LIMIT: usize = 32;
#[derive(Debug, Clone)]
struct SemanticArtefactCandidate {
    artefact: Artefact,
    embedding: Vec<f32>,
}

pub(crate) async fn select_semantic_artefacts_for_representation(
    context: &DevqlGraphqlContext,
    scope: &ResolverScope,
    query: &str,
    representation_kind: SemanticEmbeddingRepresentationKind,
) -> GraphqlResult<Vec<Artefact>> {
    let mut hits_by_representation =
        select_semantic_artefacts_by_representation(context, scope, query, &[representation_kind])
            .await?;
    Ok(hits_by_representation
        .remove(&representation_kind)
        .unwrap_or_default())
}

pub(crate) async fn select_semantic_artefacts_by_representation(
    context: &DevqlGraphqlContext,
    scope: &ResolverScope,
    query: &str,
    representation_kinds: &[SemanticEmbeddingRepresentationKind],
) -> GraphqlResult<HashMap<SemanticEmbeddingRepresentationKind, Vec<Artefact>>> {
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return Err(bad_user_input_error(
            "`selectArtefacts(by: ...)` requires a non-empty `search`",
        ));
    }
    if representation_kinds.is_empty() {
        return Ok(HashMap::new());
    }

    let repo_id = context.repo_id_for_scope(scope).map_err(|err| {
        backend_error(format!(
            "failed to resolve repository for search selection: {err:#}"
        ))
    })?;
    let host = context.capability_host_arc().map_err(|err| {
        backend_error(format!(
            "failed to resolve capability host for search selection: {err:#}"
        ))
    })?;
    let config = resolve_semantic_clones_config(&host.config_view(SEMANTIC_CLONES_CAPABILITY_ID));
    if !embeddings_enabled(&config) {
        return Ok(HashMap::new());
    }
    let inference = host.inference_for_capability(SEMANTIC_CLONES_CAPABILITY_ID);
    let representation_kinds = dedup_representation_kinds(representation_kinds);

    let relational = context
        .open_relational_storage("semantic search selection")
        .await
        .map_err(|err| {
            backend_error(format!(
                "failed to open relational storage for search selection: {err:#}"
            ))
        })?;

    let mut hits_by_representation = representation_kinds
        .iter()
        .copied()
        .map(|representation_kind| (representation_kind, Vec::new()))
        .collect::<HashMap<_, _>>();
    let mut query_embeddings_by_setup = HashMap::<String, Vec<f32>>::new();
    let vector_backend = SemanticVectorBackend::resolve(&relational);

    for representation_kind in representation_kinds {
        let selection = resolve_embedding_provider(
            &config,
            &inference,
            representation_kind,
            EmbeddingProviderMode::ConfiguredDegrade,
        )
        .map_err(|err| {
            backend_error(format!(
                "failed to resolve {representation_kind} embeddings provider for search: {err:#}"
            ))
        })?;
        let Some(provider) = selection.provider else {
            continue;
        };
        let query_setup = resolve_embedding_setup(provider.as_ref()).map_err(|err| {
            backend_error(format!(
                "failed to resolve {representation_kind} embedding setup for search: {err:#}"
            ))
        })?;
        let active_setup = load_primary_active_embedding_setup(
            &relational,
            &repo_id,
            representation_kind,
        )
        .await
        .map_err(|err| {
            backend_error(format!(
                "failed to load active {representation_kind} embedding setup for search: {err:#}"
            ))
        })?;
        if let Some(active_setup) = active_setup
            && active_setup.setup != query_setup
        {
            continue;
        }

        let embedding_cache_key = format!(
            "{}::{}",
            provider.cache_key(),
            query_setup.setup_fingerprint.as_str()
        );
        let query_embedding =
            if let Some(query_embedding) = query_embeddings_by_setup.get(&embedding_cache_key) {
                query_embedding.clone()
            } else {
                let query_embedding = provider
                    .embed(trimmed_query, EmbeddingInputType::Query)
                    .map_err(|err| {
                        backend_error(format!(
                            "failed to embed search query for {representation_kind}: {err:#}"
                        ))
                    })?;
                query_embeddings_by_setup.insert(embedding_cache_key, query_embedding.clone());
                query_embedding
            };
        if query_embedding.is_empty() {
            return Err(backend_error(format!(
                "{representation_kind} embedding provider returned an empty vector for search",
            )));
        }
        if query_embedding.len() != query_setup.dimension {
            continue;
        }

        let candidate_ids = vector_backend
            .nearest_current_candidates(SemanticVectorQuery {
                repo_id: &repo_id,
                representation_kind,
                setup_fingerprint: &query_setup.setup_fingerprint,
                dimension: query_setup.dimension,
                query_embedding: &query_embedding,
                limit: semantic_candidate_prefilter_limit(DEFAULT_RESULT_LIMIT),
            })
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to retrieve semantic vector candidates for {representation_kind}: {err:#}"
                ))
            })?
            .into_iter()
            .map(|candidate| candidate.artefact_id)
            .collect::<Vec<_>>();
        if candidate_ids.is_empty() {
            continue;
        }

        let hydrated_candidates = load_semantic_candidates_for_artefact_ids(
            context,
            scope,
            &relational,
            &repo_id,
            representation_kind,
            &query_setup,
            &candidate_ids,
        )
        .await
        .map_err(|err| {
            backend_error(format!(
                "failed to hydrate semantic candidates for {representation_kind}: {err:#}"
            ))
        })?;
        if hydrated_candidates.is_empty() {
            continue;
        }

        let hits = rank_semantic_candidates(
            &query_embedding,
            hydrated_candidates,
            VectorSearchMode::Auto,
            DEFAULT_MIN_SEMANTIC_SCORE,
            DEFAULT_RESULT_LIMIT,
        );
        hits_by_representation.insert(representation_kind, hits);
    }

    Ok(hits_by_representation)
}

async fn load_primary_active_embedding_setup(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: SemanticEmbeddingRepresentationKind,
) -> Result<
    Option<
        crate::capability_packs::semantic_clones::embeddings::ActiveEmbeddingRepresentationState,
    >,
> {
    let rows = relational
        .query_rows_primary(&format!(
            "SELECT representation_kind, provider, model, dimension, setup_fingerprint \
             FROM semantic_clone_embedding_setup_state \
             WHERE repo_id = '{repo_id}' AND {representation_predicate}",
            repo_id = esc_pg(repo_id),
            representation_predicate = match representation_kind {
                SemanticEmbeddingRepresentationKind::Code => {
                    "representation_kind IN ('code', 'baseline', 'enriched')".to_string()
                }
                SemanticEmbeddingRepresentationKind::Summary => {
                    "representation_kind IN ('summary')".to_string()
                }
                SemanticEmbeddingRepresentationKind::Identity => {
                    "representation_kind IN ('identity', 'locator')".to_string()
                }
            },
        ))
        .await?;
    Ok(rows.into_iter().find_map(|row| {
        let provider = row.get("provider").and_then(Value::as_str)?.to_string();
        let model = row.get("model").and_then(Value::as_str)?.to_string();
        let dimension = row
            .get("dimension")
            .and_then(Value::as_u64)
            .or_else(|| {
                row.get("dimension")
                    .and_then(Value::as_i64)
                    .map(|value| value.max(0) as u64)
            })
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)?;
        let setup_fingerprint = row
            .get("setup_fingerprint")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                EmbeddingSetup::new(provider.as_str(), model.as_str(), dimension).setup_fingerprint
            });
        Some(
            crate::capability_packs::semantic_clones::embeddings::ActiveEmbeddingRepresentationState::new(
                match representation_kind {
                    SemanticEmbeddingRepresentationKind::Code => {
                        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
                    }
                    SemanticEmbeddingRepresentationKind::Summary => {
                        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary
                    }
                    SemanticEmbeddingRepresentationKind::Identity => {
                        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Identity
                    }
                },
                EmbeddingSetup {
                    provider,
                    model,
                    dimension,
                    setup_fingerprint,
                },
            ),
        )
    }))
}

fn semantic_candidate_prefilter_limit(result_limit: usize) -> usize {
    result_limit
        .saturating_mul(ANN_PREFILTER_MULTIPLIER)
        .max(result_limit)
        .max(ANN_PREFILTER_MIN_LIMIT)
}

async fn load_semantic_candidates_for_artefact_ids(
    context: &DevqlGraphqlContext,
    scope: &ResolverScope,
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: SemanticEmbeddingRepresentationKind,
    query_setup: &EmbeddingSetup,
    artefact_ids: &[String],
) -> Result<Vec<SemanticArtefactCandidate>> {
    if artefact_ids.is_empty() {
        return Ok(Vec::new());
    }
    let spec = plan_graphql_artefact_query(
        repo_id,
        &context.current_branch_name(scope),
        None,
        None,
        scope,
        None,
    );
    let filtered_cte = build_filtered_artefacts_cte_sql(&spec);
    let sql = format!(
        "{filtered_cte} \
         SELECT filtered.symbol_id, filtered.artefact_id, filtered.path, filtered.language, \
                filtered.canonical_kind, filtered.language_kind, filtered.symbol_fqn, \
                filtered.parent_artefact_id, filtered.start_line, filtered.end_line, \
                filtered.start_byte, filtered.end_byte, filtered.signature, filtered.modifiers, \
                filtered.docstring, filtered.summary, filtered.embedding_representations, \
                filtered.blob_sha, filtered.content_hash, filtered.created_at, \
                se.representation_kind AS representation_kind, \
                se.setup_fingerprint, se.provider AS embedding_provider, \
                se.model AS embedding_model, se.dimension AS embedding_dimension, \
                se.embedding \
           FROM filtered \
           JOIN symbol_embeddings_current se \
             ON se.repo_id = '{repo_id}' \
            AND se.artefact_id = filtered.artefact_id \
            AND se.content_id = filtered.blob_sha \
          WHERE se.artefact_id IN ({artefact_ids}) \
            AND se.representation_kind IN ({representation_clause}) \
            AND se.setup_fingerprint = '{setup_fingerprint}' \
            AND se.dimension = {dimension} \
       ORDER BY filtered.path, COALESCE(filtered.symbol_fqn, ''), filtered.artefact_id",
        filtered_cte = filtered_cte,
        repo_id = esc_pg(repo_id),
        artefact_ids = sql_string_list_pg(artefact_ids),
        representation_clause = representation_kind
            .storage_values()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|value| format!("'{}'", esc_pg(value)))
            .collect::<Vec<_>>()
            .join(", "),
        setup_fingerprint = esc_pg(&query_setup.setup_fingerprint),
        dimension = query_setup.dimension,
    );
    let rows = relational.query_rows_primary(&sql).await?;
    rows.into_iter().map(candidate_from_row).collect()
}

fn dedup_representation_kinds(
    representation_kinds: &[SemanticEmbeddingRepresentationKind],
) -> Vec<SemanticEmbeddingRepresentationKind> {
    let mut deduped = Vec::new();
    for representation_kind in representation_kinds.iter().copied() {
        if !deduped.contains(&representation_kind) {
            deduped.push(representation_kind);
        }
    }
    deduped
}

#[cfg(test)]
fn merge_semantic_hits(candidates: Vec<Artefact>, result_limit: usize) -> Vec<Artefact> {
    if candidates.is_empty() || result_limit == 0 {
        return Vec::new();
    }

    let mut best_by_artefact_id = HashMap::new();
    for candidate in candidates {
        let artefact_id = candidate.id.to_string();
        match best_by_artefact_id.get(&artefact_id) {
            Some(existing) if compare_scored_artefacts(existing, &candidate).is_lt() => continue,
            _ => {
                best_by_artefact_id.insert(artefact_id, candidate);
            }
        }
    }

    let mut merged = best_by_artefact_id.into_values().collect::<Vec<_>>();
    merged.sort_by(compare_scored_artefacts);
    merged.truncate(result_limit);
    merged
}

#[cfg(test)]
fn compare_scored_artefacts(left: &Artefact, right: &Artefact) -> Ordering {
    right
        .score
        .unwrap_or_default()
        .partial_cmp(&left.score.unwrap_or_default())
        .unwrap_or(Ordering::Equal)
        .then_with(|| left.path.cmp(&right.path))
        .then_with(|| {
            left.symbol_fqn
                .as_deref()
                .unwrap_or_default()
                .cmp(right.symbol_fqn.as_deref().unwrap_or_default())
        })
        .then_with(|| left.id.as_str().cmp(right.id.as_str()))
}

fn rank_semantic_candidates(
    query_embedding: &[f32],
    candidates: Vec<SemanticArtefactCandidate>,
    _search_mode: VectorSearchMode,
    min_score: f32,
    result_limit: usize,
) -> Vec<Artefact> {
    if query_embedding.is_empty() || candidates.is_empty() || result_limit == 0 {
        return Vec::new();
    }

    let mut ranked = candidates
        .into_iter()
        .filter_map(|candidate| {
            let score =
                normalized_cosine_similarity(query_embedding, candidate.embedding.as_slice())?;
            (score >= min_score).then(|| RankedSemanticArtefact {
                artefact: candidate.artefact,
                score,
            })
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.artefact.path.cmp(&right.artefact.path))
            .then_with(|| {
                left.artefact
                    .symbol_fqn
                    .as_deref()
                    .unwrap_or_default()
                    .cmp(right.artefact.symbol_fqn.as_deref().unwrap_or_default())
            })
            .then_with(|| left.artefact.id.as_str().cmp(right.artefact.id.as_str()))
    });
    let mut seen_artefact_ids = HashSet::new();
    ranked
        .into_iter()
        .filter(|candidate| seen_artefact_ids.insert(candidate.artefact.id.to_string()))
        .take(result_limit)
        .map(|candidate| candidate.artefact.with_score(candidate.score as f64))
        .collect()
}

#[derive(Debug, Clone)]
struct RankedSemanticArtefact {
    artefact: Artefact,
    score: f32,
}

fn candidate_from_row(row: Value) -> Result<SemanticArtefactCandidate> {
    let artefact = artefact_from_value(&row)?;
    let embedding = embedding_from_row(&row)?;
    let dimension = positive_usize_field(&row, "embedding_dimension")?;
    if embedding.len() != dimension {
        return Err(anyhow!(
            "embedding dimension {} for artefact `{}` did not match stored dimension {}",
            embedding.len(),
            artefact.id.as_str(),
            dimension
        ));
    }

    Ok(SemanticArtefactCandidate {
        artefact,
        embedding,
    })
}

fn artefact_from_value(row: &Value) -> Result<Artefact> {
    Ok(Artefact {
        id: async_graphql::ID(string_field(row, "artefact_id")?),
        symbol_id: string_field(row, "symbol_id")?,
        path: string_field(row, "path")?,
        language: string_field(row, "language")?,
        canonical_kind: optional_canonical_kind_field(row, "canonical_kind"),
        language_kind: optional_string_field(row, "language_kind"),
        symbol_fqn: optional_string_field(row, "symbol_fqn"),
        parent_artefact_id: optional_string_field(row, "parent_artefact_id").map(async_graphql::ID),
        start_line: required_i32_field(row, "start_line")?,
        end_line: required_i32_field(row, "end_line")?,
        start_byte: required_i32_field(row, "start_byte")?,
        end_byte: required_i32_field(row, "end_byte")?,
        signature: optional_string_field(row, "signature"),
        modifiers: parse_string_array_field(row, "modifiers"),
        docstring: optional_string_field(row, "docstring"),
        summary: optional_string_field(row, "summary"),
        embedding_representations: parse_embedding_representation_field(
            row,
            "embedding_representations",
        ),
        content_hash: optional_string_field(row, "content_hash"),
        blob_sha: string_field(row, "blob_sha")?,
        created_at: parse_storage_datetime(string_field(row, "created_at")?.as_str())?,
        score: None,
        search_score: None,
        scope: ResolverScope::default(),
    })
}

fn embedding_from_row(row: &Value) -> Result<Vec<f32>> {
    let raw = row
        .get("embedding")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing string field `embedding`"))?;
    serde_json::from_str::<Vec<f32>>(raw).with_context(|| "parsing `embedding` JSON array")
}

fn string_field(row: &Value, key: &str) -> Result<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("missing string field `{key}`"))
}

fn optional_string_field(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn positive_usize_field(row: &Value, key: &str) -> Result<usize> {
    row.get(key)
        .and_then(Value::as_u64)
        .or_else(|| {
            row.get(key)
                .and_then(Value::as_i64)
                .map(|value| value.max(0) as u64)
        })
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .with_context(|| format!("missing positive integer field `{key}`"))
}

fn required_i32_field(row: &Value, key: &str) -> Result<i32> {
    row.get(key)
        .and_then(Value::as_i64)
        .map(|value| value.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
        .with_context(|| format!("missing integer field `{key}`"))
}

fn optional_canonical_kind_field(row: &Value, key: &str) -> Option<CanonicalKind> {
    row.get(key)
        .and_then(Value::as_str)
        .and_then(parse_canonical_kind)
}

fn parse_canonical_kind(value: &str) -> Option<CanonicalKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "file" => Some(CanonicalKind::File),
        "namespace" => Some(CanonicalKind::Namespace),
        "module" => Some(CanonicalKind::Module),
        "import" => Some(CanonicalKind::Import),
        "type" => Some(CanonicalKind::Type),
        "interface" => Some(CanonicalKind::Interface),
        "enum" => Some(CanonicalKind::Enum),
        "callable" => Some(CanonicalKind::Callable),
        "function" => Some(CanonicalKind::Function),
        "method" => Some(CanonicalKind::Method),
        "value" | "constant" => Some(CanonicalKind::Value),
        "variable" => Some(CanonicalKind::Variable),
        "member" => Some(CanonicalKind::Member),
        "parameter" => Some(CanonicalKind::Parameter),
        "type_parameter" => Some(CanonicalKind::TypeParameter),
        "alias" => Some(CanonicalKind::Alias),
        _ => None,
    }
}

fn parse_string_array_field(row: &Value, key: &str) -> Vec<String> {
    let Some(value) = row.get(key) else {
        return Vec::new();
    };
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
    }
    value
        .as_str()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
}

fn parse_embedding_representation_field(
    row: &Value,
    key: &str,
) -> Vec<GraphqlEmbeddingRepresentationKind> {
    let mut parsed = Vec::new();

    for value in parse_string_array_field(row, key) {
        let mapped = match value.trim().to_ascii_lowercase().as_str() {
            "identity" | "locator" => Some(GraphqlEmbeddingRepresentationKind::Identity),
            "code" | "baseline" | "enriched" => Some(GraphqlEmbeddingRepresentationKind::Code),
            "summary" => Some(GraphqlEmbeddingRepresentationKind::Summary),
            _ => None,
        };

        if let Some(kind) = mapped
            && !parsed.contains(&kind)
        {
            parsed.push(kind);
        }
    }

    parsed.sort_by_key(|kind| match kind {
        GraphqlEmbeddingRepresentationKind::Identity => 0,
        GraphqlEmbeddingRepresentationKind::Code => 1,
        GraphqlEmbeddingRepresentationKind::Summary => 2,
    });
    parsed
}

fn parse_storage_datetime(value: &str) -> Result<DateTimeScalar> {
    if let Ok(timestamp) = DateTimeScalar::from_rfc3339(value.to_string()) {
        return Ok(timestamp);
    }

    let parsed = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("parsing storage timestamp `{value}`"))?;
    let zero_offset = FixedOffset::east_opt(0).expect("zero offset is valid");
    DateTimeScalar::from_rfc3339(
        DateTime::<FixedOffset>::from_naive_utc_and_offset(parsed, zero_offset).to_rfc3339(),
    )
    .with_context(|| format!("normalising storage timestamp `{value}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql::ResolverScope;

    #[derive(Debug, Clone)]
    struct SemanticArtefactCandidateFixture {
        representation_kind: SemanticEmbeddingRepresentationKind,
        setup: EmbeddingSetup,
    }

    fn compatible_candidates_for_representation_setup(
        candidates: &[SemanticArtefactCandidateFixture],
        representation_kind: SemanticEmbeddingRepresentationKind,
        query_setup: &EmbeddingSetup,
    ) -> Vec<SemanticArtefactCandidateFixture> {
        candidates
            .iter()
            .filter(|candidate| {
                candidate.representation_kind == representation_kind
                    && candidate.setup == *query_setup
            })
            .cloned()
            .collect()
    }

    fn sample_artefact(id: &str, path: &str, symbol_fqn: &str) -> Artefact {
        Artefact {
            id: async_graphql::ID::from(id),
            symbol_id: format!("sym::{id}"),
            path: path.to_string(),
            language: "typescript".to_string(),
            canonical_kind: Some(CanonicalKind::Function),
            language_kind: Some("function_declaration".to_string()),
            symbol_fqn: Some(symbol_fqn.to_string()),
            parent_artefact_id: None,
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 30,
            signature: None,
            modifiers: Vec::new(),
            docstring: None,
            summary: None,
            embedding_representations: Vec::new(),
            content_hash: None,
            blob_sha: format!("blob::{id}"),
            created_at: DateTimeScalar::from_rfc3339("2026-04-21T09:00:00Z")
                .expect("valid timestamp"),
            score: None,
            search_score: None,
            scope: ResolverScope::default(),
        }
    }

    fn sample_candidate(
        id: &str,
        path: &str,
        symbol_fqn: &str,
        embedding: Vec<f32>,
    ) -> SemanticArtefactCandidate {
        SemanticArtefactCandidate {
            artefact: sample_artefact(id, path, symbol_fqn),
            embedding,
        }
    }

    fn sample_candidate_fixture() -> SemanticArtefactCandidateFixture {
        SemanticArtefactCandidateFixture {
            representation_kind: SemanticEmbeddingRepresentationKind::Identity,
            setup: EmbeddingSetup::new("bitloops_embeddings_ipc", "semantic-query-test-model", 3),
        }
    }

    #[test]
    fn semantic_ranking_prefers_highest_scores_with_stable_tiebreaks() {
        let ranked = rank_semantic_candidates(
            &[1.0, 0.0, 0.0],
            vec![
                sample_candidate("b", "src/b.ts", "src/b.ts::beta", vec![0.9, 0.1, 0.0]),
                sample_candidate("a", "src/a.ts", "src/a.ts::alpha", vec![0.9, 0.1, 0.0]),
                sample_candidate("c", "src/c.ts", "src/c.ts::gamma", vec![0.0, 1.0, 0.0]),
            ],
            VectorSearchMode::Exact,
            0.72,
            20,
        );

        let ids = ranked
            .into_iter()
            .map(|artefact| artefact.id.to_string())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn semantic_ranking_filters_weak_matches() {
        let ranked = rank_semantic_candidates(
            &[1.0, 0.0, 0.0],
            vec![
                sample_candidate(
                    "caller",
                    "src/caller.ts",
                    "src/caller.ts::caller",
                    vec![1.0, 0.0, 0.0],
                ),
                sample_candidate(
                    "render",
                    "src/render.ts",
                    "src/render.ts::render",
                    vec![0.0, 1.0, 0.0],
                ),
            ],
            VectorSearchMode::Exact,
            0.72,
            20,
        );

        let ids = ranked
            .into_iter()
            .map(|artefact| artefact.id.to_string())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["caller"]);
    }

    #[test]
    fn semantic_candidate_filter_skips_setup_mismatches() {
        let query_setup = EmbeddingSetup::new("provider-a", "model-a", 3);
        let mismatched = sample_candidate_fixture();

        let compatible = compatible_candidates_for_representation_setup(
            &[mismatched],
            SemanticEmbeddingRepresentationKind::Identity,
            &query_setup,
        );

        assert!(compatible.is_empty());
    }

    #[test]
    fn semantic_ranking_ann_matches_exact_for_same_fixture_set() {
        let mut candidates = (0..160)
            .map(|idx| {
                let weight = idx as f32 / 160.0;
                sample_candidate(
                    &format!("sym-{idx}"),
                    &format!("src/{idx}.ts"),
                    &format!("src/{idx}.ts::sym_{idx}"),
                    vec![weight, 1.0 - weight, 0.5],
                )
            })
            .collect::<Vec<_>>();
        candidates.push(sample_candidate(
            "best",
            "src/best.ts",
            "src/best.ts::best",
            vec![0.99, 0.01, 0.5],
        ));
        candidates.push(sample_candidate(
            "next",
            "src/next.ts",
            "src/next.ts::next",
            vec![0.98, 0.02, 0.5],
        ));

        let exact = rank_semantic_candidates(
            &[1.0, 0.0, 0.5],
            candidates.clone(),
            VectorSearchMode::Exact,
            0.0,
            10,
        );
        let ann =
            rank_semantic_candidates(&[1.0, 0.0, 0.5], candidates, VectorSearchMode::Ann, 0.0, 10);

        assert_eq!(ann, exact);
    }

    #[test]
    fn semantic_ranking_dedupes_multiple_representation_hits_for_same_artefact() {
        let ranked = rank_semantic_candidates(
            &[1.0, 0.0, 0.0],
            vec![
                sample_candidate(
                    "shared",
                    "src/user.ts",
                    "src/user.ts::User::name",
                    vec![0.99, 0.01, 0.0],
                ),
                sample_candidate(
                    "shared",
                    "src/user.ts",
                    "src/user.ts::User::name",
                    vec![0.98, 0.02, 0.0],
                ),
                sample_candidate(
                    "other",
                    "src/other.ts",
                    "src/other.ts::other",
                    vec![0.97, 0.03, 0.0],
                ),
            ],
            VectorSearchMode::Exact,
            0.72,
            20,
        );

        let ids = ranked
            .into_iter()
            .map(|artefact| artefact.id.to_string())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["shared", "other"]);
    }

    #[test]
    fn semantic_merge_keeps_highest_scoring_representation_hit_per_artefact() {
        let merged = merge_semantic_hits(
            vec![
                sample_artefact("shared", "src/user.ts", "src/user.ts::user").with_score(0.81),
                sample_artefact("shared", "src/user.ts", "src/user.ts::user").with_score(0.93),
                sample_artefact("other", "src/other.ts", "src/other.ts::other").with_score(0.9),
            ],
            5,
        );

        let ids = merged
            .into_iter()
            .map(|artefact| artefact.id.to_string())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["shared", "other"]);
    }
}
