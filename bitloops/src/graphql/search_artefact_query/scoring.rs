use std::cmp::Ordering;
use std::sync::OnceLock;

use regex::Regex;

use crate::graphql::types::{Artefact, ArtefactSearchScore};

use super::types::{
    DocumentMatchStats, FieldMatchStats, QueryMatchKind, RankedSearchArtefact,
    SearchDocumentCandidate, SearchSignal,
};

pub(super) fn build_document_full_text_signal(
    document: &SearchDocumentCandidate,
    query: &str,
    sql_signal: f64,
) -> SearchSignal {
    let stats = build_document_match_stats(document, query);
    let computed_signal = document_full_text_signal_from_stats(&stats, query_match_kind(query));
    let full_text_signal = computed_signal.max(sql_signal);

    if full_text_signal <= 0.0 {
        return SearchSignal::default();
    }

    SearchSignal {
        full_text_signal,
        literal_matches: stats.total_literal_matches(),
        exact_case_literal_matches: stats.total_exact_case_literal_matches(),
        phrase_matches: stats.total_phrase_matches(),
        exact_case_phrase_matches: stats.total_exact_case_phrase_matches(),
        body_literal_matches: stats.body.literal_matches,
        signature_literal_matches: stats.signature.literal_matches,
        summary_literal_matches: stats.summary.literal_matches,
        ..SearchSignal::default()
    }
}

fn build_document_match_stats(
    document: &SearchDocumentCandidate,
    query: &str,
) -> DocumentMatchStats {
    let kind = query_match_kind(query);
    DocumentMatchStats {
        signature: count_field_matches(document.signature_text.as_deref(), query, kind),
        summary: count_field_matches(document.summary_text.as_deref(), query, kind),
        body: count_field_matches(document.body_text.as_deref(), query, kind),
    }
}

fn count_field_matches(value: Option<&str>, query: &str, kind: QueryMatchKind) -> FieldMatchStats {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return FieldMatchStats::default();
    };
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return FieldMatchStats::default();
    }

    match kind {
        QueryMatchKind::IdentifierLiteral => FieldMatchStats {
            literal_matches: count_identifier_occurrences_case_insensitive(value, trimmed_query),
            exact_case_literal_matches: count_identifier_occurrences(value, trimmed_query),
            ..FieldMatchStats::default()
        },
        QueryMatchKind::Phrase => FieldMatchStats {
            phrase_matches: count_substring_occurrences_case_insensitive(value, trimmed_query),
            exact_case_phrase_matches: count_substring_occurrences(value, trimmed_query),
            ..FieldMatchStats::default()
        },
    }
}

fn document_full_text_signal_from_stats(stats: &DocumentMatchStats, kind: QueryMatchKind) -> f64 {
    match kind {
        QueryMatchKind::IdentifierLiteral => {
            let total_literal_matches = stats.total_literal_matches();
            if total_literal_matches == 0 {
                return 0.0;
            }

            0.6 + capped_match_score(stats.body.literal_matches, 32, 0.06)
                + capped_match_score(stats.signature.literal_matches, 16, 0.05)
                + capped_match_score(stats.summary.literal_matches, 12, 0.04)
                + capped_match_score(stats.total_exact_case_literal_matches(), 32, 0.08)
        }
        QueryMatchKind::Phrase => {
            let total_phrase_matches = stats.total_phrase_matches();
            if total_phrase_matches == 0 {
                return 0.0;
            }

            0.7 + capped_match_score(stats.body.phrase_matches, 16, 0.08)
                + capped_match_score(stats.signature.phrase_matches, 12, 0.06)
                + capped_match_score(stats.summary.phrase_matches, 10, 0.04)
                + capped_match_score(stats.total_exact_case_phrase_matches(), 16, 0.10)
        }
    }
}

fn capped_match_score(count: usize, cap: usize, weight: f64) -> f64 {
    (count.min(cap) as f64) * weight
}

fn query_match_kind(query: &str) -> QueryMatchKind {
    if identifier_literal_regex().is_match(query.trim()) {
        QueryMatchKind::IdentifierLiteral
    } else {
        QueryMatchKind::Phrase
    }
}

fn identifier_literal_regex() -> &'static Regex {
    static IDENTIFIER_LITERAL_REGEX: OnceLock<Regex> = OnceLock::new();
    IDENTIFIER_LITERAL_REGEX.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap())
}

fn count_identifier_occurrences(value: &str, query: &str) -> usize {
    count_identifier_occurrences_internal(value, query, true)
}

fn count_identifier_occurrences_case_insensitive(value: &str, query: &str) -> usize {
    count_identifier_occurrences_internal(value, query, false)
}

fn count_identifier_occurrences_internal(value: &str, query: &str, case_sensitive: bool) -> usize {
    let needle = query.trim();
    if needle.is_empty() {
        return 0;
    }

    let haystack = if case_sensitive {
        value.to_string()
    } else {
        value.to_ascii_lowercase()
    };
    let needle = if case_sensitive {
        needle.to_string()
    } else {
        needle.to_ascii_lowercase()
    };

    let mut count = 0;
    let mut offset = 0;
    while let Some(found_at) = haystack[offset..].find(needle.as_str()) {
        let start = offset + found_at;
        let end = start + needle.len();
        if is_identifier_boundary(value, start, end) {
            count += 1;
        }
        offset = end;
    }
    count
}

fn count_substring_occurrences(value: &str, query: &str) -> usize {
    count_substring_occurrences_internal(value, query, true)
}

fn count_substring_occurrences_case_insensitive(value: &str, query: &str) -> usize {
    count_substring_occurrences_internal(value, query, false)
}

fn count_substring_occurrences_internal(value: &str, query: &str, case_sensitive: bool) -> usize {
    let needle = query.trim();
    if needle.is_empty() {
        return 0;
    }

    let haystack = if case_sensitive {
        value.to_string()
    } else {
        value.to_ascii_lowercase()
    };
    let needle = if case_sensitive {
        needle.to_string()
    } else {
        needle.to_ascii_lowercase()
    };

    let mut count = 0;
    let mut offset = 0;
    while let Some(found_at) = haystack[offset..].find(needle.as_str()) {
        let end = offset + found_at + needle.len();
        count += 1;
        offset = end;
    }
    count
}

fn is_identifier_boundary(value: &str, start: usize, end: usize) -> bool {
    let previous = value[..start].chars().next_back();
    let next = value[end..].chars().next();
    !previous.is_some_and(is_identifier_char) && !next.is_some_and(is_identifier_char)
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

pub(super) fn merge_search_signal(target: &mut SearchSignal, incoming: &SearchSignal) {
    target.exact_signal = target.exact_signal.max(incoming.exact_signal);
    target.full_text_signal = target.full_text_signal.max(incoming.full_text_signal);
    target.fuzzy_signal = target.fuzzy_signal.max(incoming.fuzzy_signal);
    target.semantic_signal = target.semantic_signal.max(incoming.semantic_signal);
    target.literal_matches = target.literal_matches.max(incoming.literal_matches);
    target.exact_case_literal_matches = target
        .exact_case_literal_matches
        .max(incoming.exact_case_literal_matches);
    target.phrase_matches = target.phrase_matches.max(incoming.phrase_matches);
    target.exact_case_phrase_matches = target
        .exact_case_phrase_matches
        .max(incoming.exact_case_phrase_matches);
    target.body_literal_matches = target
        .body_literal_matches
        .max(incoming.body_literal_matches);
    target.signature_literal_matches = target
        .signature_literal_matches
        .max(incoming.signature_literal_matches);
    target.summary_literal_matches = target
        .summary_literal_matches
        .max(incoming.summary_literal_matches);
}

fn search_score_from_signal(signal: &SearchSignal) -> ArtefactSearchScore {
    let (exact, full_text, fuzzy, semantic) = search_score_components(signal);
    ArtefactSearchScore {
        total: exact + full_text + fuzzy + semantic,
        exact,
        full_text,
        fuzzy,
        semantic,
        literal_matches: saturating_search_int(signal.literal_matches),
        exact_case_literal_matches: saturating_search_int(signal.exact_case_literal_matches),
        phrase_matches: saturating_search_int(signal.phrase_matches),
        exact_case_phrase_matches: saturating_search_int(signal.exact_case_phrase_matches),
        body_literal_matches: saturating_search_int(signal.body_literal_matches),
        signature_literal_matches: saturating_search_int(signal.signature_literal_matches),
        summary_literal_matches: saturating_search_int(signal.summary_literal_matches),
    }
}

pub(super) fn search_total_from_signal(signal: &SearchSignal) -> f64 {
    let (exact, full_text, fuzzy, semantic) = search_score_components(signal);
    exact + full_text + fuzzy + semantic
}

fn search_score_components(signal: &SearchSignal) -> (f64, f64, f64, f64) {
    let exact = if signal.exact_signal > 0.0 {
        4_000.0 + (signal.exact_signal * 100.0)
    } else {
        0.0
    };
    let full_text = if signal.exact_signal > 0.0 {
        signal.full_text_signal * 50.0
    } else if signal.full_text_signal > 0.0 {
        3_000.0 + (signal.full_text_signal * 100.0)
    } else {
        0.0
    };
    let fuzzy = if signal.exact_signal > 0.0 || signal.full_text_signal > 0.0 {
        signal.fuzzy_signal * 10.0
    } else if signal.fuzzy_signal > 0.0 {
        2_000.0 + (signal.fuzzy_signal * 100.0)
    } else {
        0.0
    };
    let semantic = if signal.semantic_signal > 0.0 {
        1_000.0 + (signal.semantic_signal * 100.0)
    } else {
        0.0
    };
    (exact, full_text, fuzzy, semantic)
}

pub(super) fn finalize_hits(hits: &[RankedSearchArtefact], limit: usize) -> Vec<Artefact> {
    hits.iter()
        .take(limit)
        .cloned()
        .map(RankedSearchArtefact::into_artefact)
        .collect()
}

pub(super) fn compare_ranked_search_artefacts(
    left: &RankedSearchArtefact,
    right: &RankedSearchArtefact,
) -> Ordering {
    search_total_from_signal(&right.signal)
        .partial_cmp(&search_total_from_signal(&left.signal))
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
}

fn saturating_search_int(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

impl RankedSearchArtefact {
    fn into_artefact(self) -> Artefact {
        self.artefact
            .with_search_score(search_score_from_signal(&self.signal))
    }
}

impl DocumentMatchStats {
    fn total_literal_matches(&self) -> usize {
        self.signature.literal_matches + self.summary.literal_matches + self.body.literal_matches
    }

    fn total_exact_case_literal_matches(&self) -> usize {
        self.signature.exact_case_literal_matches
            + self.summary.exact_case_literal_matches
            + self.body.exact_case_literal_matches
    }

    fn total_phrase_matches(&self) -> usize {
        self.signature.phrase_matches + self.summary.phrase_matches + self.body.phrase_matches
    }

    fn total_exact_case_phrase_matches(&self) -> usize {
        self.signature.exact_case_phrase_matches
            + self.summary.exact_case_phrase_matches
            + self.body.exact_case_phrase_matches
    }
}
