use std::cmp::Ordering;
use std::collections::HashSet;

use anyhow::{Context as _, Result, anyhow};
use async_graphql::Result as GraphqlResult;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use serde_json::Value;

use crate::artefact_query_planner::plan_graphql_artefact_query;
use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::embeddings::{
    EmbeddingRepresentationKind, EmbeddingSetup, resolve_embedding_setup,
};
use crate::capability_packs::semantic_clones::load_active_embedding_setup;
use crate::capability_packs::semantic_clones::runtime_config::{
    EmbeddingProviderMode, resolve_embedding_provider, resolve_semantic_clones_config,
};
use crate::graphql::types::{Artefact, CanonicalKind, DateTimeScalar};
use crate::graphql::{DevqlGraphqlContext, ResolverScope, backend_error, bad_user_input_error};
use crate::host::devql::artefact_sql::build_filtered_artefacts_cte_sql;
use crate::host::devql::esc_pg;
use crate::host::inference::EmbeddingInputType;
use crate::host::relational_store::DefaultRelationalStore;
use crate::vector_search::{HnswLikeIndex, VectorSearchMode, normalized_cosine_similarity};

const DEFAULT_MIN_SEMANTIC_SCORE: f32 = 0.72;
const DEFAULT_RESULT_LIMIT: usize = 20;
const ANN_PREFILTER_MULTIPLIER: usize = 4;
const ANN_PREFILTER_MIN_LIMIT: usize = 32;

#[derive(Debug, Clone)]
struct SemanticArtefactCandidate {
    artefact: Artefact,
    embedding: Vec<f32>,
    setup: EmbeddingSetup,
}

pub(crate) async fn select_semantic_artefacts(
    context: &DevqlGraphqlContext,
    scope: &ResolverScope,
    query: &str,
) -> GraphqlResult<Vec<Artefact>> {
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return Err(bad_user_input_error(
            "`selectArtefacts(by: ...)` requires a non-empty `semanticQuery`",
        ));
    }

    let repo_id = context.repo_id_for_scope(scope).map_err(|err| {
        backend_error(format!(
            "failed to resolve repository for semanticQuery selection: {err:#}"
        ))
    })?;
    let host = context.capability_host_arc().map_err(|err| {
        backend_error(format!(
            "failed to resolve capability host for semanticQuery selection: {err:#}"
        ))
    })?;
    let config = resolve_semantic_clones_config(&host.config_view(SEMANTIC_CLONES_CAPABILITY_ID));
    let inference = host.inference_for_capability(SEMANTIC_CLONES_CAPABILITY_ID);
    let selection = resolve_embedding_provider(
        &config,
        &inference,
        EmbeddingRepresentationKind::Code,
        EmbeddingProviderMode::ConfiguredStrict,
    )
    .map_err(|err| {
        backend_error(format!(
            "failed to resolve semanticQuery embeddings provider: {err:#}"
        ))
    })?;
    let provider = selection.provider.ok_or_else(|| {
        bad_user_input_error(
            "`semanticQuery` requires configured semantic clone code embeddings in `semantic_clones.inference.code_embeddings`",
        )
    })?;
    let query_setup = resolve_embedding_setup(provider.as_ref()).map_err(|err| {
        backend_error(format!(
            "failed to resolve semanticQuery embedding setup: {err:#}"
        ))
    })?;
    let query_embedding = provider
        .embed(trimmed_query, EmbeddingInputType::Query)
        .map_err(|err| backend_error(format!("failed to embed semanticQuery: {err:#}")))?;
    if query_embedding.is_empty() {
        return Err(backend_error(
            "semanticQuery embedding provider returned an empty vector",
        ));
    }
    if query_embedding.len() != query_setup.dimension {
        return Err(bad_user_input_error(format!(
            "`semanticQuery` embedding dimension {} did not match the configured setup dimension {}",
            query_embedding.len(),
            query_setup.dimension
        )));
    }

    let repo_root = context.repo_root_for_scope(scope).map_err(|err| {
        backend_error(format!(
            "failed to resolve repository root for semanticQuery selection: {err:#}"
        ))
    })?;
    let relational_store =
        DefaultRelationalStore::open_local_for_repo_root(&repo_root).map_err(|err| {
            backend_error(format!(
                "failed to open relational store for semanticQuery selection: {err:#}"
            ))
        })?;
    let relational = relational_store.to_local_inner();

    let active_setup =
        load_active_embedding_setup(&relational, &repo_id, EmbeddingRepresentationKind::Code)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to load active embedding setup for semanticQuery selection: {err:#}"
                ))
            })?;
    if let Some(active_setup) = active_setup
        && active_setup.setup != query_setup
    {
        return Err(bad_user_input_error(format!(
            "`semanticQuery` embeddings were prepared for {} but the active inference profile resolved to {}",
            describe_setup(&active_setup.setup),
            describe_setup(&query_setup)
        )));
    }

    let candidates = load_semantic_candidates(context, scope, &repo_id)
        .await
        .map_err(|err| {
            backend_error(format!(
                "failed to load candidate artefacts for semanticQuery selection: {err:#}"
            ))
        })?;
    let compatible_candidates =
        compatible_candidates_for_setup(candidates, &query_setup).map_err(bad_user_input_error)?;
    if compatible_candidates.is_empty() {
        return Ok(Vec::new());
    }

    Ok(rank_semantic_candidates(
        &query_embedding,
        compatible_candidates,
        VectorSearchMode::Auto,
        DEFAULT_MIN_SEMANTIC_SCORE,
        DEFAULT_RESULT_LIMIT,
    ))
}

async fn load_semantic_candidates(
    context: &DevqlGraphqlContext,
    scope: &ResolverScope,
    repo_id: &str,
) -> Result<Vec<SemanticArtefactCandidate>> {
    let spec = plan_graphql_artefact_query(
        repo_id,
        &context.current_branch_name(scope),
        None,
        None,
        scope,
        None,
    );
    let filtered_cte = build_filtered_artefacts_cte_sql(&spec);
    let representation_clause = [
        EmbeddingRepresentationKind::Code,
        EmbeddingRepresentationKind::Identity,
    ]
    .into_iter()
    .flat_map(EmbeddingRepresentationKind::storage_values)
    .map(|value| format!("'{}'", esc_pg(value)))
    .collect::<Vec<_>>()
    .join(", ");
    let sql = format!(
        "{filtered_cte} \
         SELECT filtered.symbol_id, filtered.artefact_id, filtered.path, filtered.language, \
                filtered.canonical_kind, filtered.language_kind, filtered.symbol_fqn, \
                filtered.parent_artefact_id, filtered.start_line, filtered.end_line, \
                filtered.start_byte, filtered.end_byte, filtered.signature, filtered.modifiers, \
                filtered.docstring, filtered.blob_sha, filtered.content_hash, filtered.created_at, \
                se.setup_fingerprint, se.provider AS embedding_provider, \
                se.model AS embedding_model, se.dimension AS embedding_dimension, \
                se.embedding \
           FROM filtered \
           JOIN symbol_embeddings_current se \
             ON se.repo_id = '{repo_id}' \
            AND se.artefact_id = filtered.artefact_id \
            AND se.content_id = filtered.blob_sha \
          WHERE se.representation_kind IN ({representation_clause}) \
       ORDER BY filtered.path, COALESCE(filtered.symbol_fqn, ''), filtered.artefact_id",
        filtered_cte = filtered_cte,
        repo_id = esc_pg(repo_id),
        representation_clause = representation_clause,
    );
    let rows = context.query_devql_sqlite_rows(&sql).await?;
    rows.into_iter().map(candidate_from_row).collect()
}

fn compatible_candidates_for_setup(
    candidates: Vec<SemanticArtefactCandidate>,
    query_setup: &EmbeddingSetup,
) -> std::result::Result<Vec<SemanticArtefactCandidate>, String> {
    let mut compatible = Vec::new();
    let mut mismatched_setup = None;

    for candidate in candidates {
        if candidate.setup == *query_setup {
            compatible.push(candidate);
        } else if mismatched_setup.is_none() {
            mismatched_setup = Some(candidate.setup);
        }
    }

    if compatible.is_empty()
        && let Some(mismatched_setup) = mismatched_setup
    {
        return Err(format!(
            "`semanticQuery` embeddings were prepared for {} but the active inference profile resolved to {}",
            describe_setup(&mismatched_setup),
            describe_setup(query_setup)
        ));
    }

    Ok(compatible)
}

fn rank_semantic_candidates(
    query_embedding: &[f32],
    candidates: Vec<SemanticArtefactCandidate>,
    search_mode: VectorSearchMode,
    min_score: f32,
    result_limit: usize,
) -> Vec<Artefact> {
    if query_embedding.is_empty() || candidates.is_empty() || result_limit == 0 {
        return Vec::new();
    }

    let vectors = candidates
        .iter()
        .map(|candidate| candidate.embedding.clone())
        .collect::<Vec<_>>();
    let index = HnswLikeIndex::build(&vectors);
    let prefiltered_limit = if matches!(search_mode, VectorSearchMode::Exact) {
        candidates.len()
    } else {
        result_limit
            .saturating_mul(ANN_PREFILTER_MULTIPLIER)
            .max(result_limit)
            .max(ANN_PREFILTER_MIN_LIMIT)
            .min(candidates.len())
    };
    let candidate_indices =
        index.nearest_to_vector_with_mode(query_embedding, prefiltered_limit, search_mode);

    let mut ranked = candidate_indices
        .into_iter()
        .filter_map(|candidate_idx| {
            let candidate = candidates.get(candidate_idx)?;
            let score =
                normalized_cosine_similarity(query_embedding, candidate.embedding.as_slice())?;
            (score >= min_score).then(|| RankedSemanticArtefact {
                artefact: candidate.artefact.clone(),
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
        .map(|candidate| candidate.artefact)
        .collect()
}

#[derive(Debug, Clone)]
struct RankedSemanticArtefact {
    artefact: Artefact,
    score: f32,
}

fn candidate_from_row(row: Value) -> Result<SemanticArtefactCandidate> {
    let artefact = artefact_from_value(&row)?;
    let setup = embedding_setup_from_row(&row)?;
    let embedding = embedding_from_row(&row)?;
    if embedding.len() != setup.dimension {
        return Err(anyhow!(
            "embedding dimension {} for artefact `{}` did not match stored dimension {}",
            embedding.len(),
            artefact.id.as_str(),
            setup.dimension
        ));
    }

    Ok(SemanticArtefactCandidate {
        artefact,
        embedding,
        setup,
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
        content_hash: optional_string_field(row, "content_hash"),
        blob_sha: string_field(row, "blob_sha")?,
        created_at: parse_storage_datetime(string_field(row, "created_at")?.as_str())?,
        scope: ResolverScope::default(),
    })
}

fn embedding_setup_from_row(row: &Value) -> Result<EmbeddingSetup> {
    let provider = string_field(row, "embedding_provider")?;
    let model = string_field(row, "embedding_model")?;
    let dimension = positive_usize_field(row, "embedding_dimension")?;
    let mut setup = EmbeddingSetup::new(provider, model, dimension);
    if let Some(setup_fingerprint) = optional_string_field(row, "setup_fingerprint") {
        setup.setup_fingerprint = setup_fingerprint;
    }
    Ok(setup)
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

fn describe_setup(setup: &EmbeddingSetup) -> String {
    format!(
        "provider `{}` model `{}` dimension {} (setup `{}`)",
        setup.provider, setup.model, setup.dimension, setup.setup_fingerprint
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql::ResolverScope;

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
            content_hash: None,
            blob_sha: format!("blob::{id}"),
            created_at: DateTimeScalar::from_rfc3339("2026-04-21T09:00:00Z")
                .expect("valid timestamp"),
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
    fn semantic_query_setup_mismatch_returns_user_facing_error() {
        let query_setup = EmbeddingSetup::new("provider-a", "model-a", 3);
        let mismatched = sample_candidate(
            "caller",
            "src/caller.ts",
            "src/caller.ts::caller",
            vec![1.0, 0.0, 0.0],
        );

        let err = compatible_candidates_for_setup(vec![mismatched], &query_setup)
            .expect_err("mismatched setup should fail");
        assert!(err.contains("`semanticQuery` embeddings were prepared for"));
        assert!(err.contains("provider `bitloops_embeddings_ipc`"));
        assert!(err.contains("provider `provider-a`"));
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
}
