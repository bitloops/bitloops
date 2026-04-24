use std::cmp::Ordering;
use std::collections::BTreeSet;

use super::types::Artefact;

const DEFAULT_MIN_SCORE: f32 = 0.6;
const DEFAULT_RESULT_LIMIT: usize = 5;
const SINGLE_TOKEN_OVERLAP_SCORE_CAP: f32 = 0.58;
const SHORT_QUERY_SCORE_CAP: f32 = 0.57;

#[derive(Debug, Clone)]
struct RankedArtefact {
    artefact: Artefact,
    score: f32,
}

pub(crate) fn select_fuzzy_named_artefacts(query: &str, artefacts: Vec<Artefact>) -> Vec<Artefact> {
    let normalized_query = normalize_fuzzy_name(query);
    if normalized_query.is_empty() {
        return Vec::new();
    }

    let mut ranked = artefacts
        .into_iter()
        .filter_map(|artefact| {
            let candidate_name = candidate_name_from_artefact(&artefact)?;
            let score = fuzzy_name_score(&normalized_query, &candidate_name);
            (score >= DEFAULT_MIN_SCORE).then_some(RankedArtefact { artefact, score })
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.artefact.path.cmp(&right.artefact.path))
            .then_with(|| left.artefact.symbol_fqn.cmp(&right.artefact.symbol_fqn))
            .then_with(|| left.artefact.id.as_str().cmp(right.artefact.id.as_str()))
    });
    ranked.truncate(DEFAULT_RESULT_LIMIT);
    ranked
        .into_iter()
        .map(|candidate| candidate.artefact.with_score(candidate.score as f64))
        .collect()
}

fn candidate_name_from_artefact(artefact: &Artefact) -> Option<String> {
    let symbol_fqn = artefact.symbol_fqn.as_deref()?.trim();
    if symbol_fqn.is_empty() {
        return None;
    }

    let leaf_name = symbol_fqn.rsplit("::").next().unwrap_or(symbol_fqn);
    let normalized = normalize_fuzzy_name(leaf_name);
    (!normalized.is_empty()).then_some(normalized)
}

fn normalize_fuzzy_name(value: &str) -> String {
    let mut trimmed = value.trim();
    while let Some(stripped) = trimmed.strip_suffix("()") {
        trimmed = stripped.trim_end();
    }

    let tokens = split_identifier_tokens(trimmed);
    if tokens.is_empty() {
        trimmed.to_ascii_lowercase()
    } else {
        tokens.join("_")
    }
}

fn fuzzy_name_score(query: &str, candidate: &str) -> f32 {
    if query.is_empty() || candidate.is_empty() {
        return 0.0;
    }
    if query == candidate {
        return 1.0;
    }

    let query_token_list = query_tokens(query);
    let candidate_tokens = query_tokens(candidate);
    let prefix_score: f32 = if candidate.starts_with(query) {
        0.92
    } else if query.starts_with(candidate) {
        0.78
    } else {
        0.0
    };
    let contains_score: f32 = if candidate.contains(query) || query.contains(candidate) {
        if query_token_list.len() == 1
            && candidate_tokens.len() > 1
            && !candidate.starts_with(query)
        {
            0.56
        } else {
            0.84
        }
    } else {
        0.0
    };
    let edit_score = levenshtein_similarity(query, candidate);
    let token_score = jaccard_similarity(&query_token_list, &candidate_tokens);
    let token_coverage = query_token_coverage(&query_token_list, &candidate_tokens);
    let mut score = prefix_score
        .max(contains_score)
        .max(edit_score)
        .max((edit_score * 0.7) + (token_score * 0.15) + (token_coverage * 0.15));

    if query_token_list.len() == 1
        && candidate_tokens.len() > 1
        && prefix_score < 0.9
        && edit_score < 0.82
    {
        score = score.min(SINGLE_TOKEN_OVERLAP_SCORE_CAP);
    }

    let candidate_extra_tokens = candidate_tokens
        .len()
        .saturating_sub(query_token_list.len());
    if query_token_list.len() <= 2
        && candidate_extra_tokens >= 2
        && token_coverage < 1.0
        && prefix_score < 0.9
        && edit_score < 0.8
    {
        score = score.min(SHORT_QUERY_SCORE_CAP);
    }

    score
}

fn query_tokens(value: &str) -> Vec<&str> {
    value
        .split('_')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect()
}

fn jaccard_similarity(left: &[&str], right: &[&str]) -> f32 {
    if left.is_empty() && right.is_empty() {
        return 1.0;
    }
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    let left = left.iter().copied().collect::<BTreeSet<_>>();
    let right = right.iter().copied().collect::<BTreeSet<_>>();
    let shared = left.intersection(&right).count();
    let union = left.union(&right).count();
    if union == 0 {
        0.0
    } else {
        shared as f32 / union as f32
    }
}

fn query_token_coverage(query_tokens: &[&str], candidate_tokens: &[&str]) -> f32 {
    if query_tokens.is_empty() {
        return 0.0;
    }

    let candidate_tokens = candidate_tokens.iter().copied().collect::<BTreeSet<_>>();
    let shared = query_tokens
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|token| candidate_tokens.contains(token))
        .count();
    shared as f32 / query_tokens.len() as f32
}

fn levenshtein_similarity(left: &str, right: &str) -> f32 {
    let distance = levenshtein_distance(left, right);
    let max_len = left.chars().count().max(right.chars().count());
    if max_len == 0 {
        1.0
    } else {
        1.0 - (distance as f32 / max_len as f32)
    }
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    if left.is_empty() {
        return right.len();
    }
    if right.is_empty() {
        return left.len();
    }

    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];

    for (left_index, left_char) in left.iter().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right.iter().enumerate() {
            let substitution_cost = usize::from(left_char != right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution_cost);
        }
        previous.clone_from(&current);
    }

    previous[right.len()]
}

fn split_identifier_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
            continue;
        }

        flush_identifier_token(&mut current, &mut tokens);
    }
    flush_identifier_token(&mut current, &mut tokens);

    tokens
}

fn flush_identifier_token(current: &mut String, tokens: &mut Vec<String>) {
    if current.is_empty() {
        return;
    }

    for piece in split_camel_case_word(current) {
        let lowered = piece.trim().to_ascii_lowercase();
        if !lowered.is_empty() {
            tokens.push(lowered);
        }
    }

    current.clear();
}

fn split_camel_case_word(input: &str) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }

    let chars = input.chars().collect::<Vec<_>>();
    let mut pieces = Vec::new();
    let mut current = String::new();

    for (index, ch) in chars.iter().enumerate() {
        if !current.is_empty() {
            let previous = chars[index - 1];
            let next = chars.get(index + 1).copied().unwrap_or('\0');
            let boundary = (previous.is_ascii_lowercase() && ch.is_ascii_uppercase())
                || (previous.is_ascii_alphabetic() && ch.is_ascii_digit())
                || (previous.is_ascii_digit() && ch.is_ascii_alphabetic())
                || (previous.is_ascii_uppercase()
                    && ch.is_ascii_uppercase()
                    && next.is_ascii_lowercase())
                || (*ch == '_' && previous != '_');
            if boundary {
                let piece = current.trim_matches('_');
                if !piece.is_empty() {
                    pieces.push(piece.to_string());
                }
                current.clear();
            }
        }
        current.push(*ch);
    }

    let piece = current.trim_matches('_');
    if !piece.is_empty() {
        pieces.push(piece.to_string());
    }

    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql::{DateTimeScalar, ResolverScope};
    use async_graphql::ID;

    fn sample_artefact(id: &str, path: &str, symbol_fqn: &str) -> Artefact {
        Artefact {
            id: ID::from(id),
            symbol_id: format!("symbol::{id}"),
            path: path.to_string(),
            language: "typescript".to_string(),
            canonical_kind: None,
            language_kind: None,
            symbol_fqn: Some(symbol_fqn.to_string()),
            parent_artefact_id: None,
            start_line: 1,
            end_line: 5,
            start_byte: 0,
            end_byte: 10,
            signature: None,
            modifiers: Vec::new(),
            docstring: None,
            summary: None,
            embedding_representations: Vec::new(),
            content_hash: None,
            blob_sha: format!("blob::{id}"),
            created_at: DateTimeScalar::from_rfc3339("2026-04-20T09:00:00Z")
                .expect("valid timestamp"),
            score: None,
            search_score: None,
            scope: ResolverScope::default(),
        }
    }

    #[test]
    fn normalize_fuzzy_name_splits_identifiers_and_strips_call_syntax() {
        assert_eq!(normalize_fuzzy_name(" payLater() "), "pay_later");
        assert_eq!(normalize_fuzzy_name("HTTPServer_v2"), "http_server_v_2");
    }

    #[test]
    fn fuzzy_name_selection_prefers_best_typo_match() {
        let selected = select_fuzzy_named_artefacts(
            "targte()",
            vec![
                sample_artefact(
                    "file-target",
                    "packages/api/src/target.ts",
                    "packages/api/src/target.ts",
                ),
                sample_artefact(
                    "target",
                    "packages/api/src/target.ts",
                    "packages/api/src/target.ts::target",
                ),
                sample_artefact(
                    "caller",
                    "packages/api/src/caller.ts",
                    "packages/api/src/caller.ts::caller",
                ),
            ],
        );

        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].symbol_fqn.as_deref(),
            Some("packages/api/src/target.ts::target")
        );
    }

    #[test]
    fn fuzzy_name_selection_filters_weak_matches() {
        let selected = select_fuzzy_named_artefacts(
            "payments()",
            vec![sample_artefact(
                "caller",
                "packages/api/src/caller.ts",
                "packages/api/src/caller.ts::caller",
            )],
        );

        assert!(selected.is_empty());
    }

    #[test]
    fn fuzzy_name_selection_caps_results_at_five() {
        let artefacts = (0..12)
            .map(|index| {
                sample_artefact(
                    &format!("pay-later-{index}"),
                    &format!("src/pay-later-{index}.ts"),
                    &format!("src/pay-later-{index}.ts::payLaterVariant{index}"),
                )
            })
            .collect::<Vec<_>>();

        let selected = select_fuzzy_named_artefacts("payLater()", artefacts);

        assert_eq!(selected.len(), 5);
    }

    #[test]
    fn fuzzy_name_selection_rejects_single_token_overlap_without_prefix_support() {
        let selected = select_fuzzy_named_artefacts(
            "target",
            vec![sample_artefact(
                "payload-builder",
                "src/payload-builder.ts",
                "src/payload-builder.ts::buildTargetPayload",
            )],
        );

        assert!(selected.is_empty());
    }

    #[test]
    fn fuzzy_name_selection_keeps_prefix_matches_for_longer_identifiers() {
        let selected = select_fuzzy_named_artefacts(
            "target",
            vec![sample_artefact(
                "target-builder",
                "src/target-builder.ts",
                "src/target-builder.ts::targetPayloadBuilder",
            )],
        );

        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].symbol_fqn.as_deref(),
            Some("src/target-builder.ts::targetPayloadBuilder")
        );
    }

    #[test]
    fn jaccard_similarity_uses_set_semantics_for_duplicate_tokens() {
        let score = jaccard_similarity(&["foo", "foo"], &["foo"]);

        assert_eq!(score, 1.0);
    }
}
