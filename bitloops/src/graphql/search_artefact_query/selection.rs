use std::collections::HashMap;
use std::path::Path;

use async_graphql::Result as GraphqlResult;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::graphql::fuzzy_artefact_name::select_fuzzy_named_artefacts;
use crate::graphql::semantic_artefact_query::{
    select_semantic_artefacts_by_representation, select_semantic_artefacts_for_representation,
};
use crate::graphql::types::{Artefact, SearchBreakdown, SearchMode};
use crate::graphql::{DevqlGraphqlContext, ResolverScope, backend_error, bad_user_input_error};
use crate::host::devql::RelationalStorage;

use super::lexical::{
    build_exact_lexical_hits, build_full_text_hits, build_source_slice_full_text_hits,
};
use super::scoring::{
    compare_ranked_search_artefacts, finalize_hits, merge_search_signal, search_total_from_signal,
};
use super::storage::{load_search_documents, load_search_features};
use super::types::{
    RankedSearchArtefact, SearchArtefactBundle, SearchDocumentCandidate, SearchFeatureCandidate,
    SearchSignal,
};
use super::{SEARCH_BREAKDOWN_LIMIT, SEARCH_CANDIDATE_LIMIT, SEARCH_RESULT_LIMIT};

struct LexicalSearchStorageContext<'a> {
    relational: &'a RelationalStorage,
    repo_id: &'a str,
    artefact_ids: &'a [String],
    repo_root: Option<&'a Path>,
}

pub(crate) async fn select_search_artefacts(
    context: &DevqlGraphqlContext,
    scope: &ResolverScope,
    query: &str,
    mode: SearchMode,
) -> GraphqlResult<SearchArtefactBundle> {
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return Err(bad_user_input_error(
            "`selectArtefacts(by: ...)` requires a non-empty `search`",
        ));
    }

    let artefacts = context
        .list_artefacts(None, None, scope)
        .await
        .map_err(|err| {
            backend_error(format!(
                "failed to resolve selected artefacts by search: {err:#}"
            ))
        })?;
    if artefacts.is_empty() {
        return Ok(SearchArtefactBundle {
            unified: Vec::new(),
            breakdown: (mode == SearchMode::Auto).then_some(SearchBreakdown {
                lexical: Vec::new(),
                identity: Vec::new(),
                code: Vec::new(),
                summary: Vec::new(),
            }),
        });
    }

    let repo_id = context.repo_id_for_scope(scope).map_err(|err| {
        backend_error(format!(
            "failed to resolve repository for search selection: {err:#}"
        ))
    })?;
    let artefact_ids = artefacts
        .iter()
        .map(|artefact| artefact.id.to_string())
        .collect::<Vec<_>>();
    let artefacts_by_id = artefacts
        .iter()
        .cloned()
        .map(|artefact| (artefact.id.to_string(), artefact))
        .collect::<HashMap<_, _>>();
    let repo_root = context.repo_root_for_scope(scope).ok();

    let relational = context
        .open_relational_storage("GraphQL hybrid artefact search")
        .await
        .map_err(|err| {
            backend_error(format!(
                "failed to open relational storage for search selection: {err:#}"
            ))
        })?;
    let features_by_id = load_search_features(&relational, &repo_id, &artefact_ids)
        .await
        .map_err(|err| {
            backend_error(format!(
                "failed to load lexical search features for search selection: {err:#}"
            ))
        })?;
    let documents_by_id = load_search_documents(&relational, &repo_id, &artefact_ids)
        .await
        .map_err(|err| {
            backend_error(format!(
                "failed to load full-text search documents for search selection: {err:#}"
            ))
        })?;
    let lexical_storage = LexicalSearchStorageContext {
        relational: &relational,
        repo_id: &repo_id,
        artefact_ids: &artefact_ids,
        repo_root: repo_root.as_deref(),
    };

    let lexical_hits = if matches!(mode, SearchMode::Auto | SearchMode::Lexical) {
        build_lexical_hits(
            trimmed_query,
            &artefacts,
            &artefacts_by_id,
            &features_by_id,
            &documents_by_id,
            &lexical_storage,
        )
        .await?
    } else {
        Vec::new()
    };

    let (identity_hits, code_hits, summary_hits) = if mode == SearchMode::Auto {
        let semantic_hits = select_semantic_artefacts_by_representation(
            context,
            scope,
            trimmed_query,
            &[
                EmbeddingRepresentationKind::Identity,
                EmbeddingRepresentationKind::Code,
                EmbeddingRepresentationKind::Summary,
            ],
        )
        .await?;
        (
            ranked_semantic_hits(
                semantic_hits
                    .get(&EmbeddingRepresentationKind::Identity)
                    .cloned()
                    .unwrap_or_default(),
            ),
            ranked_semantic_hits(
                semantic_hits
                    .get(&EmbeddingRepresentationKind::Code)
                    .cloned()
                    .unwrap_or_default(),
            ),
            ranked_semantic_hits(
                semantic_hits
                    .get(&EmbeddingRepresentationKind::Summary)
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
    } else {
        let identity_hits = if mode == SearchMode::Identity {
            semantic_hits_for_mode(
                context,
                scope,
                trimmed_query,
                EmbeddingRepresentationKind::Identity,
            )
            .await?
        } else {
            Vec::new()
        };
        let code_hits = if mode == SearchMode::Code {
            semantic_hits_for_mode(
                context,
                scope,
                trimmed_query,
                EmbeddingRepresentationKind::Code,
            )
            .await?
        } else {
            Vec::new()
        };
        let summary_hits = if mode == SearchMode::Summary {
            semantic_hits_for_mode(
                context,
                scope,
                trimmed_query,
                EmbeddingRepresentationKind::Summary,
            )
            .await?
        } else {
            Vec::new()
        };
        (identity_hits, code_hits, summary_hits)
    };

    let unified = match mode {
        SearchMode::Auto => finalize_hits(
            &merge_auto_hits(
                &lexical_hits,
                &identity_hits,
                &code_hits,
                &summary_hits,
                SEARCH_RESULT_LIMIT,
            ),
            SEARCH_RESULT_LIMIT,
        ),
        SearchMode::Lexical => finalize_hits(&lexical_hits, SEARCH_RESULT_LIMIT),
        SearchMode::Identity => finalize_hits(&identity_hits, SEARCH_RESULT_LIMIT),
        SearchMode::Code => finalize_hits(&code_hits, SEARCH_RESULT_LIMIT),
        SearchMode::Summary => finalize_hits(&summary_hits, SEARCH_RESULT_LIMIT),
    };

    let breakdown = (mode == SearchMode::Auto).then(|| SearchBreakdown {
        lexical: finalize_hits(&lexical_hits, SEARCH_BREAKDOWN_LIMIT),
        identity: finalize_hits(&identity_hits, SEARCH_BREAKDOWN_LIMIT),
        code: finalize_hits(&code_hits, SEARCH_BREAKDOWN_LIMIT),
        summary: finalize_hits(&summary_hits, SEARCH_BREAKDOWN_LIMIT),
    });

    Ok(SearchArtefactBundle { unified, breakdown })
}

async fn build_lexical_hits(
    query: &str,
    artefacts: &[Artefact],
    artefacts_by_id: &HashMap<String, Artefact>,
    features_by_id: &HashMap<String, SearchFeatureCandidate>,
    documents_by_id: &HashMap<String, SearchDocumentCandidate>,
    storage: &LexicalSearchStorageContext<'_>,
) -> GraphqlResult<Vec<RankedSearchArtefact>> {
    let exact_hits = build_exact_lexical_hits(query, artefacts_by_id, features_by_id);
    let mut full_text_hits = build_full_text_hits(
        storage.relational,
        storage.repo_id,
        storage.artefact_ids,
        artefacts_by_id,
        documents_by_id,
        query,
    )
    .await
    .map_err(|err| {
        backend_error(format!(
            "failed to execute lexical full-text search for selection: {err:#}"
        ))
    })?;
    let source_slice_hits =
        build_source_slice_full_text_hits(query, storage.repo_root, &exact_hits, artefacts_by_id);
    full_text_hits.extend(source_slice_hits);
    let fuzzy_hits = select_fuzzy_named_artefacts(query, artefacts.to_vec())
        .into_iter()
        .map(|artefact| RankedSearchArtefact {
            signal: SearchSignal {
                fuzzy_signal: artefact.score.unwrap_or_default(),
                ..SearchSignal::default()
            },
            artefact,
        })
        .collect();

    Ok(merge_lexical_hits(
        artefacts_by_id,
        exact_hits,
        full_text_hits,
        fuzzy_hits,
        SEARCH_CANDIDATE_LIMIT,
    ))
}

fn merge_lexical_hits(
    artefacts_by_id: &HashMap<String, Artefact>,
    exact_hits: Vec<RankedSearchArtefact>,
    full_text_hits: Vec<RankedSearchArtefact>,
    fuzzy_hits: Vec<RankedSearchArtefact>,
    limit: usize,
) -> Vec<RankedSearchArtefact> {
    let mut signal_by_artefact = HashMap::<String, SearchSignal>::new();
    for hit in exact_hits
        .into_iter()
        .chain(full_text_hits)
        .chain(fuzzy_hits)
    {
        merge_search_signal(
            signal_by_artefact
                .entry(hit.artefact.id.to_string())
                .or_default(),
            &hit.signal,
        );
    }

    let mut merged = signal_by_artefact
        .into_iter()
        .filter_map(|(artefact_id, signal)| {
            (search_total_from_signal(&signal) > 0.0)
                .then(|| artefacts_by_id.get(&artefact_id).cloned())
                .flatten()
                .map(|artefact| RankedSearchArtefact { artefact, signal })
        })
        .collect::<Vec<_>>();
    merged.sort_by(compare_ranked_search_artefacts);
    merged.truncate(limit);
    merged
}

fn merge_auto_hits(
    lexical_hits: &[RankedSearchArtefact],
    identity_hits: &[RankedSearchArtefact],
    code_hits: &[RankedSearchArtefact],
    summary_hits: &[RankedSearchArtefact],
    limit: usize,
) -> Vec<RankedSearchArtefact> {
    let mut artefacts_by_id = HashMap::<String, Artefact>::new();
    let mut signal_by_artefact = HashMap::<String, SearchSignal>::new();

    for hit in lexical_hits
        .iter()
        .chain(code_hits.iter())
        .chain(summary_hits.iter())
        .chain(identity_hits.iter())
    {
        artefacts_by_id
            .entry(hit.artefact.id.to_string())
            .or_insert_with(|| hit.artefact.clone());
        merge_search_signal(
            signal_by_artefact
                .entry(hit.artefact.id.to_string())
                .or_default(),
            &hit.signal,
        );
    }

    let mut merged = signal_by_artefact
        .into_iter()
        .filter_map(|(artefact_id, signal)| {
            artefacts_by_id
                .remove(&artefact_id)
                .map(|artefact| RankedSearchArtefact { artefact, signal })
        })
        .collect::<Vec<_>>();

    merged.sort_by(compare_ranked_search_artefacts);
    merged.truncate(limit);
    merged
}

async fn semantic_hits_for_mode(
    context: &DevqlGraphqlContext,
    scope: &ResolverScope,
    query: &str,
    representation_kind: EmbeddingRepresentationKind,
) -> GraphqlResult<Vec<RankedSearchArtefact>> {
    let raw_hits =
        select_semantic_artefacts_for_representation(context, scope, query, representation_kind)
            .await?;
    Ok(ranked_semantic_hits(raw_hits))
}

fn ranked_semantic_hits(raw_hits: Vec<Artefact>) -> Vec<RankedSearchArtefact> {
    raw_hits
        .into_iter()
        .map(|artefact| RankedSearchArtefact {
            signal: SearchSignal {
                semantic_signal: artefact.score.unwrap_or_default(),
                ..SearchSignal::default()
            },
            artefact,
        })
        .collect()
}
