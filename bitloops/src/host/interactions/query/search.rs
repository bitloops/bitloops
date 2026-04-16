use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};

use super::filters::{
    session_matches_filter, session_sort_key, turn_matches_filter, turn_sort_key,
};
use super::state::load_state;
use super::types::{InteractionSearchInput, InteractionSessionSearchHit, InteractionTurnSearchHit};
use crate::host::interactions::db_store::SqliteInteractionSpool;

const FIELD_PROMPT: &str = "prompt";
const FIELD_SUMMARY: &str = "summary";
const FIELD_TOOL: &str = "tool";
const FIELD_PATH: &str = "path";
const FIELD_TRANSCRIPT: &str = "transcript";

pub(crate) fn search_session_summaries(
    repo_root: &Path,
    input: &InteractionSearchInput,
) -> Result<Vec<InteractionSessionSearchHit>> {
    let state = load_state(repo_root)?;
    let summaries = state
        .session_summaries
        .into_values()
        .filter(|summary| session_matches_filter(summary, &input.filter))
        .collect::<Vec<_>>();
    let allowed = summaries
        .iter()
        .map(|summary| summary.session.session_id.clone())
        .collect::<HashSet<_>>();
    let score_map = score_documents(
        state.spool.as_ref(),
        &state.repo_id,
        &input.query,
        &allowed,
        SearchDocumentKind::Session,
    )?;
    let mut hits = summaries
        .into_iter()
        .filter_map(|summary| {
            score_map
                .get(&summary.session.session_id)
                .map(|score| InteractionSessionSearchHit {
                    session: summary,
                    score: score.score,
                    matched_fields: score.matched_fields.clone(),
                })
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| session_sort_key(&right.session).cmp(&session_sort_key(&left.session)))
    });
    let limit = search_limit(input.limit);
    hits.truncate(limit);
    Ok(hits)
}

pub(crate) fn search_turn_summaries(
    repo_root: &Path,
    input: &InteractionSearchInput,
) -> Result<Vec<InteractionTurnSearchHit>> {
    let state = load_state(repo_root)?;
    let session_summaries = state.session_summaries.clone();
    let turns = state
        .turn_summaries
        .into_values()
        .filter(|turn| turn_matches_filter(turn, &input.filter))
        .collect::<Vec<_>>();
    let allowed = turns
        .iter()
        .map(|turn| turn.turn.turn_id.clone())
        .collect::<HashSet<_>>();
    let score_map = score_documents(
        state.spool.as_ref(),
        &state.repo_id,
        &input.query,
        &allowed,
        SearchDocumentKind::Turn,
    )?;
    let mut hits = turns
        .into_iter()
        .filter_map(|turn| {
            let score = score_map.get(&turn.turn.turn_id)?;
            let session = session_summaries.get(&turn.turn.session_id)?.clone();
            Some(InteractionTurnSearchHit {
                turn,
                session,
                score: score.score,
                matched_fields: score.matched_fields.clone(),
            })
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| turn_sort_key(&right.turn).cmp(&turn_sort_key(&left.turn)))
    });
    let limit = search_limit(input.limit);
    hits.truncate(limit);
    Ok(hits)
}

fn search_limit(limit: usize) -> usize {
    limit.clamp(1, 200)
}

#[derive(Clone, Copy)]
enum SearchDocumentKind {
    Session,
    Turn,
}

struct SearchScore {
    score: i64,
    matched_fields: Vec<String>,
}

fn score_documents(
    spool: Option<&SqliteInteractionSpool>,
    repo_id: &str,
    query: &str,
    allowed_ids: &HashSet<String>,
    kind: SearchDocumentKind,
) -> Result<HashMap<String, SearchScore>> {
    let Some(spool) = spool else {
        return Ok(HashMap::new());
    };
    let terms = tokenise(query);
    if terms.is_empty() {
        return Ok(allowed_ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    SearchScore {
                        score: 0,
                        matched_fields: Vec::new(),
                    },
                )
            })
            .collect());
    }
    spool.with_connection(|conn| {
        let id_column = match kind {
            SearchDocumentKind::Session => "session_id",
            SearchDocumentKind::Turn => "turn_id",
        };
        let table = match kind {
            SearchDocumentKind::Session => "interaction_session_search_terms",
            SearchDocumentKind::Turn => "interaction_turn_search_terms",
        };
        let placeholders = (0..terms.len())
            .map(|index| format!("?{}", index + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {id_column}, field, SUM(occurrences) AS occurrences
             FROM {table}
             WHERE repo_id = ?1 AND term IN ({placeholders})
             GROUP BY {id_column}, field"
        );
        let mut params: Vec<&dyn rusqlite::types::ToSql> = vec![&repo_id];
        for term in &terms {
            params.push(term);
        }
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("reading interaction lexical search scores")?;
        let mut scores: HashMap<String, SearchScore> = HashMap::new();
        for (document_id, field, occurrences) in rows {
            if !allowed_ids.contains(&document_id) {
                continue;
            }
            let score = field_weight(field.as_str()) * occurrences;
            let entry = scores.entry(document_id).or_insert_with(|| SearchScore {
                score: 0,
                matched_fields: Vec::new(),
            });
            entry.score += score;
            if !entry
                .matched_fields
                .iter()
                .any(|existing| existing == &field)
            {
                entry.matched_fields.push(field);
            }
        }
        Ok(scores)
    })
}

fn field_weight(field: &str) -> i64 {
    match field {
        FIELD_PROMPT => 8,
        FIELD_SUMMARY => 5,
        FIELD_TOOL => 5,
        FIELD_PATH => 3,
        FIELD_TRANSCRIPT => 1,
        _ => 1,
    }
}

fn tokenise(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in input.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}
