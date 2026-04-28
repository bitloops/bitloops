use crate::capability_packs::semantic_clones::features::render_dependency_context;

use super::identity::{
    identity_container_raw, normalize_identity_path, normalize_identity_path_terms,
    normalize_identity_terms,
};
use super::types::{EmbeddingRepresentationKind, SymbolEmbeddingInput};

pub(super) const MAX_EMBEDDING_BODY_CHARS: usize = 8_000;

pub fn build_symbol_embedding_text(input: &SymbolEmbeddingInput) -> String {
    match input.representation_kind {
        EmbeddingRepresentationKind::Code => build_code_embedding_text(input),
        EmbeddingRepresentationKind::Summary => build_summary_embedding_text(input),
        EmbeddingRepresentationKind::Identity => build_identity_embedding_text(input),
    }
}

fn build_code_embedding_text(input: &SymbolEmbeddingInput) -> String {
    let body = truncate_chars(normalize_whitespace(&input.body), MAX_EMBEDDING_BODY_CHARS);
    let signature = input
        .signature
        .as_deref()
        .map(normalize_whitespace)
        .unwrap_or_default();
    let dependencies = render_dependency_context(&input.dependency_signals);

    format!(
        "kind: {kind}\n\
language: {language}\n\
language_kind: {language_kind}\n\
name: {name}\n\
signature: {signature}\n\
dependencies: {dependencies}\n\
body:\n{body}",
        kind = input.canonical_kind,
        language = input.language,
        language_kind = input.language_kind,
        name = input.name,
        signature = signature,
        dependencies = dependencies,
        body = body,
    )
}

fn build_summary_embedding_text(input: &SymbolEmbeddingInput) -> String {
    format!(
        "kind: {kind}\n\
language: {language}\n\
name: {name}\n\
summary: {summary}",
        kind = input.canonical_kind,
        language = input.language,
        name = input.name,
        summary = normalize_whitespace(&input.summary),
    )
}

fn build_identity_embedding_text(input: &SymbolEmbeddingInput) -> String {
    let container = identity_container_raw(input);
    let path = normalize_identity_path(&input.path);
    format!(
        "kind: {kind}\n\
language: {language}\n\
name: {name}\n\
name_terms: {name_terms}\n\
container: {container}\n\
container_terms: {container_terms}\n\
path: {path}\n\
path_terms: {path_terms}",
        kind = input.canonical_kind,
        language = input.language,
        name = input.name,
        name_terms = normalize_identity_terms(&input.name),
        container = container,
        container_terms = normalize_identity_terms(&container),
        path = path,
        path_terms = normalize_identity_path_terms(&path),
    )
}

pub(super) fn truncate_chars(input: String, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        input
    } else {
        input.chars().take(max_chars).collect::<String>()
    }
}

pub(super) fn normalize_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}
