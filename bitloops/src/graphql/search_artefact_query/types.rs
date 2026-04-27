use std::collections::BTreeSet;

use crate::graphql::types::{Artefact, SearchBreakdown};

#[derive(Debug, Clone)]
pub(crate) struct SearchArtefactBundle {
    pub unified: Vec<Artefact>,
    pub breakdown: Option<SearchBreakdown>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct SearchFeatureCandidate {
    pub normalized_name: Option<String>,
    pub normalized_signature: Option<String>,
    pub identifier_tokens: BTreeSet<String>,
    pub normalized_body_tokens: BTreeSet<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct SearchDocumentCandidate {
    pub signature_text: Option<String>,
    pub summary_text: Option<String>,
    pub body_text: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct SearchSignal {
    pub exact_signal: f64,
    pub full_text_signal: f64,
    pub fuzzy_signal: f64,
    pub semantic_signal: f64,
    pub literal_matches: usize,
    pub exact_case_literal_matches: usize,
    pub phrase_matches: usize,
    pub exact_case_phrase_matches: usize,
    pub body_literal_matches: usize,
    pub signature_literal_matches: usize,
    pub summary_literal_matches: usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct FieldMatchStats {
    pub literal_matches: usize,
    pub exact_case_literal_matches: usize,
    pub phrase_matches: usize,
    pub exact_case_phrase_matches: usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DocumentMatchStats {
    pub signature: FieldMatchStats,
    pub summary: FieldMatchStats,
    pub body: FieldMatchStats,
}

#[derive(Debug, Clone)]
pub(super) struct RankedSearchArtefact {
    pub artefact: Artefact,
    pub signal: SearchSignal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QueryMatchKind {
    IdentifierLiteral,
    Phrase,
}
