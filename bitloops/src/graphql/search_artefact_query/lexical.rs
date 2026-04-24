use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;

use crate::graphql::types::Artefact;
use crate::host::devql::{RelationalDialect, RelationalStorage, esc_pg, sql_string_list_pg};

use super::scoring::{build_document_full_text_signal, compare_ranked_search_artefacts};
use super::storage::{numeric_field, query_rows_all_safe, string_field};
use super::types::{
    RankedSearchArtefact, SearchDocumentCandidate, SearchFeatureCandidate, SearchSignal,
};
use super::{POSTGRES_SIMILARITY_THRESHOLD, SEARCH_CANDIDATE_LIMIT};

pub(super) fn build_exact_lexical_hits(
    query: &str,
    artefacts_by_id: &HashMap<String, Artefact>,
    features_by_id: &HashMap<String, SearchFeatureCandidate>,
) -> Vec<RankedSearchArtefact> {
    let normalized_query = normalize_identifier_query(query);
    let normalized_signature_query = normalize_signature_query(query);
    let query_tokens = tokenize_identifier_like(query)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let lower_query = query.trim().to_ascii_lowercase();

    let mut ranked = Vec::new();
    for (artefact_id, features) in features_by_id {
        let Some(artefact) = artefacts_by_id.get(artefact_id) else {
            continue;
        };

        let symbol_fqn_exact = artefact
            .symbol_fqn
            .as_deref()
            .map(|value| value.trim().eq_ignore_ascii_case(query.trim()))
            .unwrap_or(false);
        let name_exact = features
            .normalized_name
            .as_deref()
            .is_some_and(|value| value == normalized_query);
        let signature_exact = features
            .normalized_signature
            .as_deref()
            .is_some_and(|value| value == normalized_signature_query);
        let identifier_exact = !query_tokens.is_empty()
            && query_tokens
                .iter()
                .all(|token| features.identifier_tokens.contains(token));
        let body_exact = !query_tokens.is_empty()
            && query_tokens
                .iter()
                .all(|token| features.normalized_body_tokens.contains(token));

        let exact_score = if symbol_fqn_exact {
            1.0
        } else if name_exact {
            0.99
        } else if signature_exact {
            0.97
        } else if identifier_exact && body_exact {
            0.96
        } else if identifier_exact {
            0.95
        } else if body_exact {
            0.94
        } else if !lower_query.is_empty()
            && features
                .normalized_signature
                .as_deref()
                .map(|value| value.to_ascii_lowercase())
                .is_some_and(|value| value == lower_query)
        {
            0.93
        } else {
            0.0
        };

        if exact_score > 0.0 {
            ranked.push(RankedSearchArtefact {
                artefact: artefact.clone(),
                signal: SearchSignal {
                    exact_signal: exact_score,
                    ..SearchSignal::default()
                },
            });
        }
    }

    ranked.sort_by(compare_ranked_search_artefacts);
    ranked
}

pub(super) async fn build_full_text_hits(
    relational: &RelationalStorage,
    repo_id: &str,
    artefact_ids: &[String],
    artefacts_by_id: &HashMap<String, Artefact>,
    documents_by_id: &HashMap<String, SearchDocumentCandidate>,
    query: &str,
) -> Result<Vec<RankedSearchArtefact>> {
    if artefact_ids.is_empty() || documents_by_id.is_empty() {
        return Ok(Vec::new());
    }

    let sql_hits = match relational.dialect() {
        RelationalDialect::Sqlite => {
            build_sqlite_full_text_hits(relational, repo_id, artefact_ids, query).await?
        }
        RelationalDialect::Postgres => {
            build_postgres_full_text_hits(relational, repo_id, artefact_ids, query).await?
        }
    };

    let mut sql_signal_by_artefact = HashMap::<String, f64>::new();
    for (artefact_id, score) in sql_hits {
        sql_signal_by_artefact
            .entry(artefact_id)
            .and_modify(|existing| *existing = existing.max(score))
            .or_insert(score);
    }

    let mut hits = documents_by_id
        .iter()
        .filter_map(|(artefact_id, document)| {
            let artefact = artefacts_by_id.get(artefact_id)?.clone();
            let signal = build_document_full_text_signal(
                document,
                query,
                sql_signal_by_artefact
                    .get(artefact_id)
                    .copied()
                    .unwrap_or_default(),
            );
            (signal.full_text_signal > 0.0).then_some(RankedSearchArtefact { artefact, signal })
        })
        .collect::<Vec<_>>();
    hits.sort_by(compare_ranked_search_artefacts);
    hits.truncate(SEARCH_CANDIDATE_LIMIT);
    Ok(hits)
}

async fn build_sqlite_full_text_hits(
    relational: &RelationalStorage,
    repo_id: &str,
    artefact_ids: &[String],
    query: &str,
) -> Result<Vec<(String, f64)>> {
    let mut hits = Vec::new();
    if let Some(fts_query) = build_sqlite_fts_query(query) {
        let sql = format!(
            "SELECT artefact_id, bm25(symbol_search_documents_current_fts) AS rank \
             FROM symbol_search_documents_current_fts \
             WHERE repo_id = '{repo_id}' \
               AND artefact_id IN ({artefact_ids}) \
               AND symbol_search_documents_current_fts MATCH '{fts_query}' \
             ORDER BY rank ASC \
             LIMIT {limit}",
            repo_id = esc_pg(repo_id),
            artefact_ids = sql_string_list_pg(artefact_ids),
            fts_query = esc_pg(&fts_query),
            limit = SEARCH_CANDIDATE_LIMIT,
        );
        for row in query_rows_all_safe(relational, &sql).await? {
            if let Some(artefact_id) = string_field(&row, "artefact_id") {
                let raw_rank = numeric_field(&row, "rank").unwrap_or_default().abs();
                hits.push((artefact_id, 1.0 / (1.0 + raw_rank)));
            }
        }
    }

    let raw_sql = format!(
        "SELECT artefact_id, \
            CASE \
                WHEN instr(lower(COALESCE(body_text, '')), lower('{query}')) > 0 THEN 1.0 \
                WHEN instr(lower(COALESCE(signature_text, '')), lower('{query}')) > 0 THEN 0.97 \
                WHEN instr(lower(COALESCE(summary_text, '')), lower('{query}')) > 0 THEN 0.94 \
                ELSE 0.0 \
            END AS score \
         FROM symbol_search_documents_current \
         WHERE repo_id = '{repo_id}' \
           AND artefact_id IN ({artefact_ids}) \
           AND (
                instr(lower(COALESCE(body_text, '')), lower('{query}')) > 0
             OR instr(lower(COALESCE(signature_text, '')), lower('{query}')) > 0
             OR instr(lower(COALESCE(summary_text, '')), lower('{query}')) > 0
           )
         ORDER BY score DESC, artefact_id \
         LIMIT {limit}",
        query = esc_pg(query),
        repo_id = esc_pg(repo_id),
        artefact_ids = sql_string_list_pg(artefact_ids),
        limit = SEARCH_CANDIDATE_LIMIT,
    );
    for row in query_rows_all_safe(relational, &raw_sql).await? {
        if let Some(artefact_id) = string_field(&row, "artefact_id") {
            hits.push((
                artefact_id,
                numeric_field(&row, "score").unwrap_or_default(),
            ));
        }
    }

    Ok(hits)
}

async fn build_postgres_full_text_hits(
    relational: &RelationalStorage,
    repo_id: &str,
    artefact_ids: &[String],
    query: &str,
) -> Result<Vec<(String, f64)>> {
    let weighted_vector = "setweight(to_tsvector('simple', COALESCE(signature_text, '')), 'A') || \
setweight(to_tsvector('simple', COALESCE(summary_text, '')), 'B') || \
setweight(to_tsvector('simple', COALESCE(body_text, '')), 'C')";
    let escaped_query = esc_pg(query);
    let sql = format!(
        "SELECT artefact_id, GREATEST(
             ts_rank_cd(({weighted_vector}), websearch_to_tsquery('simple', '{escaped_query}')),
             CASE
                 WHEN POSITION(lower('{escaped_query}') IN lower(COALESCE(searchable_text, ''))) > 0 THEN 1.0
                 ELSE 0.0
             END,
             similarity(COALESCE(searchable_text, ''), '{escaped_query}')
         ) AS score
         FROM symbol_search_documents_current
         WHERE repo_id = '{repo_id}'
           AND artefact_id IN ({artefact_ids})
           AND (
                ({weighted_vector}) @@ websearch_to_tsquery('simple', '{escaped_query}')
             OR POSITION(lower('{escaped_query}') IN lower(COALESCE(searchable_text, ''))) > 0
             OR similarity(COALESCE(searchable_text, ''), '{escaped_query}') > {similarity_threshold}
           )
         ORDER BY score DESC, artefact_id
         LIMIT {limit}",
        repo_id = esc_pg(repo_id),
        artefact_ids = sql_string_list_pg(artefact_ids),
        similarity_threshold = POSTGRES_SIMILARITY_THRESHOLD,
        limit = SEARCH_CANDIDATE_LIMIT,
    );
    let rows = query_rows_all_safe(relational, &sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some((
                string_field(&row, "artefact_id")?,
                numeric_field(&row, "score").unwrap_or_default(),
            ))
        })
        .collect())
}

pub(super) fn build_source_slice_full_text_hits(
    query: &str,
    repo_root: Option<&Path>,
    exact_hits: &[RankedSearchArtefact],
    artefacts_by_id: &HashMap<String, Artefact>,
) -> Vec<RankedSearchArtefact> {
    let Some(repo_root) = repo_root else {
        return Vec::new();
    };

    let mut file_cache = HashMap::<PathBuf, String>::new();
    let mut hits = Vec::new();

    for hit in exact_hits {
        let artefact = artefacts_by_id
            .get(hit.artefact.id.as_str())
            .cloned()
            .unwrap_or_else(|| hit.artefact.clone());
        let absolute_path = repo_root.join(&artefact.path);
        let file_content = file_cache
            .entry(absolute_path.clone())
            .or_insert_with(|| fs::read_to_string(&absolute_path).unwrap_or_default());
        if file_content.is_empty() {
            continue;
        }

        let body_text = slice_file_content_by_lines(
            file_content.as_str(),
            artefact.start_line,
            artefact.end_line,
        );
        if body_text.trim().is_empty() {
            continue;
        }

        let signal = build_document_full_text_signal(
            &SearchDocumentCandidate {
                signature_text: artefact.signature.clone(),
                summary_text: artefact.summary.clone(),
                body_text: Some(body_text),
            },
            query,
            0.0,
        );
        if signal.full_text_signal > 0.0 {
            hits.push(RankedSearchArtefact { artefact, signal });
        }
    }

    hits
}

fn slice_file_content_by_lines(content: &str, start_line: i32, end_line: i32) -> String {
    if start_line <= 0 || end_line < start_line {
        return String::new();
    }

    let lines = content.lines().collect::<Vec<_>>();
    let start_index = usize::try_from(start_line.saturating_sub(1)).unwrap_or(usize::MAX);
    let end_index = usize::try_from(end_line).unwrap_or(usize::MAX);
    if start_index >= lines.len() {
        return String::new();
    }

    lines[start_index..end_index.min(lines.len())].join("\n")
}

fn build_sqlite_fts_query(query: &str) -> Option<String> {
    let tokens = tokenize_identifier_like(query);
    (!tokens.is_empty()).then(|| tokens.join(" AND "))
}

fn normalize_identifier_query(query: &str) -> String {
    let tokens = tokenize_identifier_like(query);
    if tokens.is_empty() {
        String::new()
    } else {
        tokens.join("_")
    }
}

fn normalize_signature_query(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_ascii_lowercase()
}

fn tokenize_identifier_like(query: &str) -> Vec<String> {
    let regex = identifier_regex();
    let mut tokens = Vec::new();
    for capture in regex.find_iter(query) {
        for piece in split_camel_case_word(capture.as_str()) {
            let token = piece.trim().to_ascii_lowercase();
            if !token.is_empty() {
                tokens.push(token);
            }
        }
    }
    tokens.sort();
    tokens.dedup();
    tokens
}

fn identifier_regex() -> &'static Regex {
    static IDENTIFIER_REGEX: OnceLock<Regex> = OnceLock::new();
    IDENTIFIER_REGEX.get_or_init(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap())
}

fn split_camel_case_word(input: &str) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }

    let chars = input.chars().collect::<Vec<_>>();
    let mut pieces = Vec::new();
    let mut current = String::new();

    for (idx, ch) in chars.iter().enumerate() {
        if !current.is_empty() {
            let prev = chars[idx - 1];
            let next = chars.get(idx + 1).copied().unwrap_or('\0');
            let boundary = (prev.is_ascii_lowercase() && ch.is_ascii_uppercase())
                || (prev.is_ascii_alphabetic() && ch.is_ascii_digit())
                || (prev.is_ascii_digit() && ch.is_ascii_alphabetic())
                || (prev.is_ascii_uppercase()
                    && ch.is_ascii_uppercase()
                    && next.is_ascii_lowercase());
            if boundary {
                pieces.push(current.clone());
                current.clear();
            }
        }
        current.push(*ch);
    }

    if !current.is_empty() {
        pieces.push(current);
    }

    pieces
}
